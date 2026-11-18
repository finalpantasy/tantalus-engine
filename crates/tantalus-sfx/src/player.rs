//! Plays one effect for the battle view: a fresh plugin per effect, the
//! battle's units behind its host callback, and each frame's packets as a
//! [`RenderList`].
//!
//! What the host does is Memoria's `SFX.BattleCallback` and
//! `BattleCallbackWithBtl` (`Global/SFX/SFX.cs`) over the units the caller
//! gives: positions, angles, sizes and bone points answer the plugin's
//! queries, VRAM calls go to the effect's [`Vram`], and the effect and
//! figure points, the background intensity and the sounds are recorded
//! for the caller. The battle setup is `SFX.InitBattleParty` and the
//! request `BTL_VFX_REQ.SetupVfxRequest`.
//!
//! The view's camera goes into the plugin before each frame, where
//! `SFX_Update` reads it (`xmmword_180069730` and the projection distance
//! at `0x180069750`), so the packets line up with the models; the
//! plugin's own camera script (`SFX_UpdateCamera`) is not run here.
//!
//! Bones are not modeled: every bone answers the unit's middle (half its
//! height above its feet) with no rotation, bone 0 its feet.

use std::sync::{Arc, Mutex};

use tantalus_assets::pc::battle_camera::PsxCamera;

use crate::image::PluginImage;
use crate::plugin::rt::Memory;
use crate::plugin::{Host, Plugin};
use crate::render::{RenderList, render_list};
use crate::vram::{Rect, Vram};

/// One bone's place now, as callback 14 (`SFX.BattleCallback`, "Get Bone
/// Stance") hands it to the plugin: the world matrix of the model's
/// `bone<NNN>` object, in the PSX form (`PSXMAT`: nine 4096ths and the
/// translation, y down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bone {
    pub r: [i16; 9],
    pub t: [i32; 3],
}

impl Bone {
    /// The identity matrix `SFX.BattleCallback` answers for a bone the model
    /// does not have (`Matrix4x4.identity`).
    pub const IDENTITY: Bone = Bone {
        r: [-4096, 0, 0, 0, 4096, 0, 0, 0, 4096],
        t: [0; 3],
    };

    /// From a Unity `localToWorldMatrix` (`m[row][column]`, y up), by the
    /// reference's own scalings and casts (`SFXData.BattleCallback`, case 14).
    pub fn from_world(m: [[f32; 4]; 4]) -> Bone {
        Bone {
            r: [
                (m[0][0] * -4096.0) as i16,
                (m[0][1] * -4096.0) as i16,
                (m[0][2] * 4096.0) as i16,
                (m[1][0] * -4096.0) as i16,
                (m[1][1] * 4096.0) as i16,
                (m[1][2] * -4096.0) as i16,
                (m[2][0] * 4096.0) as i16,
                (m[2][1] * 4096.0) as i16,
                (m[2][2] * 4096.0) as i16,
            ],
            t: [m[0][3] as i32, (-m[1][3]) as i32, m[2][3] as i32],
        }
    }
}

/// One battle unit as the plugin sees it (PSX battle space: y down).
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// `btl_id`: 1, 2, 4, 8 for the party, 0x10 and up for enemies.
    pub btl_id: u16,
    pub player: bool,
    /// `bi.slot_no` (players: the character index).
    pub slot: u8,
    /// `getSerialNumber` for players.
    pub serial: u8,
    /// Feet, now.
    pub position: [i16; 3],
    /// Battle position.
    pub base: [i16; 3],
    /// Facing, 4096 to a turn (`rot.eulerAngles.y * 11.38`).
    pub yaw: i16,
    /// Model height.
    pub height: i16,
    /// `radius_collision` (`enemy_radius`; `geo_height` is twice it).
    pub radius: i32,
    /// `geo_radius`: a player's `radius_collision`, an enemy the first
    /// enemy's `radius_effect`.
    pub geo_radius: i32,
    pub tar_bone: u8,
    /// The posed model's bones by number (`bone000`...), `None` for one the
    /// model lacks. Empty when the model is not known: callback 14 then
    /// answers the unit's feet (bone 0) or middle.
    pub bones: std::sync::Arc<Vec<Option<Bone>>>,
    /// `GeoAnim.geoAnimGetNumFrames` of each motion by `PlayerMotionIndex`
    /// (`round(length * frameRate) + 1`), and for Freya her three jump
    /// clips after the 34 (see [`JUMP_MOTIONS`]). Empty when not known:
    /// callback 9 then answers 1, as for a missing clip.
    pub motion_frames: std::sync::Arc<Vec<u16>>,
}

/// Where Freya's jump clips sit in [`Unit::motion_frames`]: `SFX.BattleCallback`
/// case 12 plays `ANH_MAIN_B0_011_110`, `_111` and `_112` for `MP_IDLE_TO_CHANT`,
/// `MP_CHANT` and `MP_MAGIC` while Jump (393) or Spear (388) plays.
pub const JUMP_MOTIONS: usize = 34;

/// `PlayerMotionIndex.MP_IDLE_CMD`: a caster's motion when its effect starts
/// (`CMD_MODE_SELECT_VFX` waits for the command stance).
const MP_IDLE_CMD: u8 = 9;

/// Who casts on whom.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    /// `btl_list` order: party, then enemies (eight at most).
    pub units: Vec<Unit>,
    pub caster: usize,
    pub targets: Vec<usize>,
    /// An item (`flgs |= 2`: the Potion's "Item" use).
    pub item: bool,
    /// `monbone[0]`, `monbone[1]` and `arg0` of the request (an enemy
    /// attack's `Vfx` op arguments; 0 for party commands).
    pub args: [i16; 3],
    /// `PatAddr[PatNum].Monster[0]` x and z.
    pub put: [i16; 2],
}

/// What the effect told the host, by frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Events {
    /// Callback 23 (`btl_cmd.ExecVfxCommand`: the HP change) per unit.
    pub effect_points: Vec<(u32, u16)>,
    /// Callback 24 (`btl2d.Btl2dReq`: the damage numbers) per unit.
    pub figure_points: Vec<(u32, u16)>,
    /// Callback 32: sound commands by frame (`arg0` mode, `arg1` sound number, `arg2`
    /// channel tag, `arg3` the `sce` flag `SFX.AdjustSoundIndex` reads). Callback 125
    /// (`SFX.soundFPS`, the sound pitch) is kept here as mode 125 with the value in `arg1`.
    pub sounds: Vec<(u32, [i32; 4])>,
}

/// What the effect has done to a unit's model since it began: the
/// plugin's `Set Position` (2), `Set Angles` (4), `Set Size` (6), `Set
/// Hidden` (18) and `Show/Hide mesh` (25) commands. The view draws the unit
/// by it while the effect plays.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UnitChange {
    /// `BTL_DATA.pos` as the plugin set it (PSX space: y down).
    pub position: Option<[i16; 3]>,
    /// `BTL_DATA.rot.eulerAngles` as the plugin set it, in degrees (the model
    /// stands at x 0, z 180).
    pub euler: Option<[f32; 3]>,
    /// `localScale`, 1 unchanged.
    pub scale: Option<[f32; 3]>,
    /// `bi.disappear`: not drawn when non-zero.
    pub disappear: u8,
    /// Every mesh and the weapon hidden (`HideMesh(65535)`).
    pub meshes_hidden: bool,
    /// The unit's `mesh_banish` meshes hidden, not its weapon
    /// (`HideMesh(mesh_banish, isVanish: true)`, Vanish).
    pub vanished: bool,
    /// The "semi-transparent fade" color (`SFX.fade`) and whether it also
    /// makes the model translucent (`GEO_POLYFLAGS_TRANS_100_PLUS_25`).
    pub fade: Option<(i32, bool)>,
    /// Brought back by `Set Hidden` off (command 18, arg 0) over every hide
    /// of no higher priority: Spear shows Freya again while she falls,
    /// though the battle still has her in the air (`SetDisappear(false, 2)`).
    pub appeared: bool,
    /// The animation the effect set (command 12), see [`EffectMotion`].
    pub motion: Option<EffectMotion>,
}

/// `Set Current Animation` (`SFX.BattleCallback` case 12: `btl_mot.setMotion`,
/// looping, frame 0) and `Stop Animation` (case 13: `bi.stop_anim`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectMotion {
    /// The effect that set it (`SFX.currentEffectID`: Jump and Spear swap
    /// Freya's chant clips for her jump clips).
    pub effect: i32,
    /// `PlayerMotionIndex` (`-1`, the default idle, is kept as 0).
    pub index: u8,
    /// The plugin frame it was set in.
    pub set: u32,
    /// `(frame, stopped)` for each `Stop Animation` since.
    pub stops: Vec<(u32, bool)>,
    /// The plugin frames played so far.
    pub now: u32,
}

impl EffectMotion {
    /// Frames the animation has advanced by: one per plugin frame after the
    /// one that set it, none while stopped.
    pub fn frames(&self) -> u32 {
        (self.set + 1..=self.now)
            .filter(|&frame| {
                !self
                    .stops
                    .iter()
                    .rev()
                    .find(|(at, _)| *at < frame)
                    .is_some_and(|(_, stopped)| *stopped)
            })
            .count() as u32
    }
}

impl UnitChange {
    /// Nothing the view would draw differently.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// `SFX.BattleCallback` over [`Setup::units`].
#[derive(Debug, Default)]
pub struct BattleHost {
    pub vram: Vram,
    pub units: Vec<Unit>,
    /// The effect (`SFX.currentEffectID`).
    pub effect: i32,
    /// What the effect did to each unit, by `btl_id`.
    pub changes: std::collections::BTreeMap<u16, UnitChange>,
    /// The background capture asked for this frame (command 120).
    pub background: Option<[i32; 7]>,
    pub frame: u32,
    pub events: Events,
    /// `battlebg.nf_SetBbgIntensity` (128 is full brightness).
    pub intensity: u8,
    /// Every `LoadImage`.
    pub uploads: Vec<crate::loader::Upload>,
}

/// The memory a callback's pointer is into: the translated plugin's, or
/// (for the DLL comparison) the process's own.
pub trait Access {
    fn read(&self, at: u64, len: usize) -> Option<Vec<u8>>;
    fn write(&mut self, at: u64, bytes: &[u8]);
}

impl Access for Memory {
    fn read(&self, at: u64, len: usize) -> Option<Vec<u8>> {
        self.bytes(at, len).ok().map(<[u8]>::to_vec)
    }

    fn write(&mut self, at: u64, bytes: &[u8]) {
        let _ = self.write_bytes(at, bytes);
    }
}

fn write_i16(memory: &mut impl Access, at: u64, values: &[i16]) {
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    memory.write(at, &bytes);
}

fn words(memory: &impl Access, at: u64, count: usize) -> Vec<u16> {
    memory
        .read(at, count * 2)
        .map(|bytes| {
            bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect()
        })
        .unwrap_or_default()
}

impl BattleHost {
    fn unit(&self, btl_id: u32) -> Option<&Unit> {
        self.units
            .iter()
            .find(|unit| u32::from(unit.btl_id) == btl_id)
    }

    /// `SFX.BattleCallback`.
    pub fn answer(&mut self, memory: &mut impl Access, code: i32, args: [i32; 4], p: u64) -> i32 {
        let command = (code as u32 >> 24) as u8;
        let [a0, a1, a2, a3] = args;
        let rect = Rect {
            x: a0 as i16,
            y: a1 as i16,
            w: a2 as i16,
            h: a3 as i16,
        };
        match command {
            100 if p != 0 => {
                let count = (i32::from(rect.w).max(0) * i32::from(rect.h).max(0)) as usize;
                let pixels = words(memory, p, count);
                self.vram.load_image(rect, &pixels);
                self.uploads.push(crate::loader::Upload { rect, pixels });
                return 0;
            }
            101 if p != 0 => {
                let pixels = self.vram.store_image(rect);
                let bytes: Vec<u8> = pixels.iter().flat_map(|v| v.to_le_bytes()).collect();
                memory.write(p, &bytes);
                return 0;
            }
            102 if p != 0 => {
                let source = words(memory, p, 4);
                self.vram.move_image(
                    Rect {
                        x: source[0] as i16,
                        y: source[1] as i16,
                        w: source[2] as i16,
                        h: source[3] as i16,
                    },
                    a0 as i16,
                    a1 as i16,
                );
                return 0;
            }
            32 => {
                self.events.sounds.push((self.frame, [a0, a1, a2, a3]));
                return 0;
            }
            // `SFX.soundFPS` (callback 125): kept with the sound commands as mode 125, the
            // value in `arg1`.
            125 => {
                self.events.sounds.push((self.frame, [125, a0, 0, 0]));
                return 0;
            }
            // `PSXTextureMgr.isBgCapture` and `bgParam`.
            120 if p != 0 => {
                let bytes = memory.read(p, 28).unwrap_or_default();
                let mut params = [0i32; 7];
                for (param, word) in params.iter_mut().zip(bytes.chunks_exact(4)) {
                    *param = i32::from_le_bytes([word[0], word[1], word[2], word[3]]);
                }
                self.background = Some(params);
                return 0;
            }
            // Camera setting (Auto), cursor shown, start type, camera number.
            111 | 115 | 122 | 124 => return 0,
            117 => return i32::from(self.intensity),
            118 => {
                self.intensity = a0 as u8;
                return 0;
            }
            121 => {
                return self
                    .units
                    .iter()
                    .filter(|unit| unit.player)
                    .fold(0, |flags, unit| flags | i32::from(unit.btl_id));
            }
            123 => return self.units.iter().filter(|unit| unit.player).count() as i32,
            100..=125 => return 0,
            _ => {}
        }
        let btl_id = code as u32 & 0xFF;
        let Some(unit) = self.unit(btl_id).cloned() else {
            // `BattleCallbackWithBtl`: no such unit.
            return i32::from(command == 9);
        };
        let change = self.changes.get(&unit.btl_id).cloned().unwrap_or_default();
        let moved = change.position.unwrap_or(unit.position);
        let middle = [moved[0], moved[1].saturating_sub(unit.height / 2), moved[2]];
        match command {
            1 if p != 0 => {
                let at = if a0 & 1 == 0 { moved } else { unit.base };
                if a0 >= 2 {
                    write_i16(memory, p, &[at[0]]);
                    write_i16(memory, p + 4, &[at[2]]);
                } else {
                    write_i16(memory, p, &at);
                }
                0
            }
            2 if p != 0 => {
                // `Set Position`: `pos = (x, -(y + attachOffset), z)`.
                if let Some(at) = words(memory, p, 3).get(0..3) {
                    self.changes.entry(unit.btl_id).or_default().position =
                        Some([at[0] as i16, at[1] as i16, at[2] as i16]);
                }
                0
            }
            3 if p != 0 => {
                // Degrees to 4096ths of a turn, as the cast the reference does.
                let ticks = |degrees: f32| (degrees.rem_euclid(360.0) * 11.377_778) as i16;
                let base_yaw = f32::from(unit.yaw) * 360.0 / 4096.0;
                let angles = match (a0, change.euler) {
                    (0, None) => [0, 0, 0],
                    // The base angle less the current one.
                    (0, Some(e)) => [
                        ticks(0.0) - ticks(e[0]),
                        ticks(base_yaw) - ticks(e[1]),
                        ticks(180.0) - ticks(e[2]),
                    ],
                    (1, Some(e)) => [ticks(e[0]), ticks(e[1]), ticks(e[2] - 180.0)],
                    _ => [0, unit.yaw, 0],
                };
                if a0 == 3 {
                    write_i16(memory, p, &[angles[0]]);
                    write_i16(memory, p + 4, &[angles[2]]);
                } else {
                    write_i16(memory, p, &angles);
                }
                0
            }
            4 => {
                // `Set Angles`: to the base angle, the base angle turned half
                // way round, or a value (`* 360 / 4096`, the turn a half
                // turn off, the roll a half turn on).
                let base_yaw = f32::from(unit.yaw) * 360.0 / 4096.0;
                let entry = self.changes.entry(unit.btl_id).or_default();
                match a0 {
                    0 => entry.euler = None,
                    1 => entry.euler = Some([0.0, base_yaw + 180.0, 180.0]),
                    2 if p != 0 => {
                        if let Some(v) = words(memory, p, 3).get(0..3) {
                            let degrees = |value: u16| f32::from(value as i16) * 0.087_890_625;
                            entry.euler =
                                Some([degrees(v[0]), degrees(v[1]) - 180.0, degrees(v[2]) + 180.0]);
                        }
                    }
                    _ => {}
                }
                0
            }
            5 if p != 0 => {
                let size = change
                    .scale
                    .map_or([4096; 3], |scale| scale.map(|v| (v * 4096.0) as i32));
                let bytes: Vec<u8> = size.iter().flat_map(|v| v.to_le_bytes()).collect();
                memory.write(p, &bytes);
                0
            }
            6 => {
                // `Set Size`: bit 128 keeps the geo scale flag (else the
                // scale goes back to 1), bit 1 sets the size. Atomos'
                // late frames only reset (`SFX.FixScaleReset`).
                let late = (self.effect == 184 && self.frame > 350)
                    || (self.effect == 446 && self.frame > 150);
                let entry = self.changes.entry(unit.btl_id).or_default();
                if late {
                    entry.scale = None;
                } else {
                    if a0 & 128 == 0 {
                        entry.scale = None;
                    }
                    if a0 & 1 == 1 {
                        entry.scale = Some([a1, a2, a3].map(|v| v as f32 / 4096.0));
                    }
                }
                0
            }
            // `Set Current Animation`; `Stop Animation` (answers 0 like the
            // oracle hosts, not the old `stop_anim`).
            12 => {
                let (effect, frame) = (self.effect, self.frame);
                self.changes.entry(unit.btl_id).or_default().motion = Some(EffectMotion {
                    effect,
                    index: u8::try_from(a0).unwrap_or(0),
                    set: frame + 1,
                    stops: Vec::new(),
                    now: frame + 1,
                });
                0
            }
            13 => {
                let frame = self.frame;
                if let Some(motion) = self
                    .changes
                    .get_mut(&unit.btl_id)
                    .and_then(|change| change.motion.as_mut())
                {
                    motion.stops.push((frame + 1, a0 != 0));
                }
                0
            }
            // `Is Hidden`.
            17 => i32::from(change.disappear != 0),
            18 => {
                // Antlion's Sinkhole freezes the caster instead.
                if self.effect != 301 {
                    // `BTL_DATA.SetDisappear(arg0 != 0, priority)`.
                    let priority = match self.effect {
                        393 | 388 | 389 => 2,
                        460 => 3,
                        _ => 1,
                    };
                    let entry = self.changes.entry(unit.btl_id).or_default();
                    entry.disappear = if a0 != 0 {
                        entry.appeared = false;
                        entry.disappear.max(priority)
                    } else if priority >= entry.disappear {
                        entry.appeared = true;
                        0
                    } else {
                        entry.disappear
                    };
                }
                0
            }
            25 => {
                let entry = self.changes.entry(unit.btl_id).or_default();
                match a0 {
                    // `HideMesh(65535)`.
                    0 => entry.meshes_hidden = true,
                    // `HideMesh(mesh_banish, isVanish)`: the view applies the unit's mask (a
                    // party member's is 65535, `btl_init.cs:415`; an enemy's is its type's
                    // `Mesh[1]`); the weapon stays.
                    1 => entry.vanished = true,
                    // `ShowMesh(65535)`.
                    2 => {
                        entry.meshes_hidden = false;
                        entry.vanished = false;
                    }
                    3 => entry.fade = Some((a1, false)),
                    4 => {
                        let value = if a1 >= 256 { a1 - 256 } else { a1 };
                        entry.fade = Some((value, value < 70));
                    }
                    _ => {}
                }
                0
            }
            // `btl_stat.SetPresentColor`: the status color again, the fade gone.
            27 => {
                if let Some(entry) = self.changes.get_mut(&unit.btl_id) {
                    entry.fade = None;
                }
                0
            }
            // `Get Current Animation's frame count` (`GeoAnim.geoAnimGetNumFrames`,
            // at least 1): the plugin waits on it for each caster motion.
            9 => {
                let index = change
                    .motion
                    .as_ref()
                    .map_or(if unit.player { MP_IDLE_CMD } else { 0 }, |motion| {
                        motion.index
                    });
                let mut index = usize::from(index);
                if matches!(self.effect, 388 | 393)
                    && (26..=28).contains(&index)
                    && unit.motion_frames.len() >= JUMP_MOTIONS + 3
                {
                    index = JUMP_MOTIONS + index - 26;
                }
                unit.motion_frames
                    .get(index)
                    .copied()
                    .filter(|&frames| frames > 0)
                    .map_or(1, i32::from)
            }
            14 => {
                // The model's own bone when its pose is known; a missing
                // bone is `Matrix4x4.identity`.
                let bone = (!unit.bones.is_empty()).then(|| {
                    usize::try_from(a1)
                        .ok()
                        .and_then(|bone| unit.bones.get(bone).copied().flatten())
                        .unwrap_or(Bone::IDENTITY)
                });
                // A moved unit carries its bones along.
                let shift = [0, 1, 2].map(|i| i32::from(moved[i]) - i32::from(unit.position[i]));
                let bone = bone.map(|mut bone| {
                    for (t, by) in bone.t.iter_mut().zip(shift) {
                        *t += by;
                    }
                    bone
                });
                let at = match bone {
                    Some(bone) => bone.t.map(|v| v as i16),
                    None if a1 == 0 => moved,
                    None => middle,
                };
                match a0 {
                    0 if p != 0 => write_i16(memory, p, &at),
                    1 => {
                        return match bone {
                            Some(bone) => bone.t[1],
                            None => i32::from(at[1]),
                        };
                    }
                    2 if p != 0 => {
                        // `PSXMAT`: r, pad, t.
                        let r = bone.map_or(Bone::IDENTITY.r, |bone| bone.r);
                        let t = bone.map_or_else(|| at.map(i32::from), |bone| bone.t);
                        let mut bytes = Vec::with_capacity(32);
                        for value in r.iter().copied().chain([0]) {
                            bytes.extend_from_slice(&value.to_le_bytes());
                        }
                        for value in t {
                            bytes.extend_from_slice(&value.to_le_bytes());
                        }
                        memory.write(p, &bytes);
                    }
                    _ => {}
                }
                0
            }
            15 => 1,
            23 => {
                self.events.effect_points.push((self.frame, unit.btl_id));
                0
            }
            24 => {
                self.events.figure_points.push((self.frame, unit.btl_id));
                0
            }
            _ => 0,
        }
    }
}

/// The host handed to the plugin; the player keeps the other handle.
struct Shared(Arc<Mutex<BattleHost>>);

impl Host for Shared {
    fn callback(
        &mut self,
        memory: &mut Memory,
        code: i32,
        a0: i32,
        a1: i32,
        a2: i32,
        a3: i32,
        p: u64,
    ) -> i32 {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .answer(memory, code, [a0, a1, a2, a3], p)
    }
}

/// `SFX_INIT_PARAM` (`SFX.InitBattleParty`, `InitBtlData`).
pub fn init_param(setup: &Setup) -> Vec<u8> {
    let mut param = vec![0u8; crate::INIT_PARAM_SIZE];
    param[4..6].copy_from_slice(&setup.put[0].to_le_bytes());
    param[6..8].copy_from_slice(&setup.put[1].to_le_bytes());
    for (index, unit) in setup.units.iter().take(8).enumerate() {
        let at = 8 + index * crate::BTL_DATA_INIT_SIZE;
        let record = &mut param[at..at + crate::BTL_DATA_INIT_SIZE];
        record[0] = u8::from(unit.player);
        record[1] = unit.slot.min(8);
        record[4] = unit.tar_bone;
        record[6] = if unit.player { unit.serial } else { 0 };
        let radius = unit.radius;
        record[8..12].copy_from_slice(&radius.to_le_bytes());
        record[12..16].copy_from_slice(&unit.geo_radius.to_le_bytes());
        record[16..20].copy_from_slice(&(radius * 2).to_le_bytes());
        record[20..22].copy_from_slice(&unit.btl_id.to_le_bytes());
    }
    param
}

/// `S_Eff_Req_Org` (`BTL_VFX_REQ.SetupVfxRequest`).
pub fn request(setup: &Setup) -> Vec<u8> {
    let mut request = vec![0u8; crate::REQUEST_SIZE];
    let id = |index: usize| setup.units.get(index).map_or(0, |unit| unit.btl_id);
    request[0..2].copy_from_slice(&id(setup.caster).to_le_bytes());
    let targets: Vec<usize> = setup.targets.iter().copied().take(8).collect();
    for (slot, &target) in targets.iter().enumerate() {
        let at = 8 + slot * 4;
        request[at..at + 2].copy_from_slice(&id(target).to_le_bytes());
    }
    request[56] = targets.len() as u8;
    if !targets.is_empty() {
        let (mut x, mut z) = (0i32, 0i32);
        for &target in &targets {
            if let Some(unit) = setup.units.get(target) {
                x += i32::from(unit.base[0]);
                z += i32::from(unit.base[2]);
            }
        }
        let n = targets.len() as i32;
        request[60..62].copy_from_slice(&((x / n) as i16).to_le_bytes());
        request[64..66].copy_from_slice(&((z / n) as i16).to_le_bytes());
    }
    request[58] = setup.args[0] as u8;
    request[59] = setup.args[1] as u8;
    request[66..68].copy_from_slice(&setup.args[2].to_le_bytes());
    let flags: u16 = if setup.item { 2 } else { 0 };
    request[68..70].copy_from_slice(&flags.to_le_bytes());
    request
}

/// One effect playing.
pub struct EffectRun {
    plugin: Plugin,
    host: Arc<Mutex<BattleHost>>,
    effect: i32,
    frame: u32,
    done: bool,
}

impl std::fmt::Debug for EffectRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectRun")
            .field("effect", &self.effect)
            .field("frame", &self.frame)
            .field("done", &self.done)
            .finish()
    }
}

/// `SFX_Update` copies these into the frame's GTE state.
pub const CAMERA_MATRIX: u64 = 0x1_8006_9730;
pub const CAMERA_DISTANCE: u64 = 0x1_8006_9750;

/// `camera` as written at [`CAMERA_MATRIX`] (rotation, pad, translation)
/// and [`CAMERA_DISTANCE`].
pub fn camera_bytes(camera: &PsxCamera) -> (Vec<u8>, [u8; 2]) {
    let mut matrix = Vec::with_capacity(32);
    for row in camera.rotation {
        for value in row {
            matrix.extend_from_slice(&(value as i16).to_le_bytes());
        }
    }
    matrix.extend_from_slice(&[0, 0]);
    for value in camera.translation {
        matrix.extend_from_slice(&value.to_le_bytes());
    }
    (matrix, (camera.h as i16).to_le_bytes())
}

fn guarded<T>(work: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).ok()
}

impl EffectRun {
    /// `SFX_InitSystem`, `SFX_InitBattle` and `SFX_Play` on a fresh plugin.
    /// `None` when the image does not load or the plugin faults.
    pub fn start(image: &PluginImage, effect: i32, bytes: &[u8], setup: &Setup) -> Option<Self> {
        let host = Arc::new(Mutex::new(BattleHost {
            units: setup.units.clone(),
            effect,
            intensity: 128,
            ..Default::default()
        }));
        let mut plugin = Plugin::load(image, Box::new(Shared(host.clone())))?;
        let param = init_param(setup);
        let request = request(setup);
        guarded(|| {
            plugin.init_system();
            plugin.init_battle(&param);
            plugin.play(effect, bytes, &request);
        })?;
        Some(Self {
            plugin,
            host,
            effect,
            frame: 0,
            done: false,
        })
    }

    /// Frames played so far.
    pub fn frame(&self) -> u32 {
        self.frame
    }

    pub fn done(&self) -> bool {
        self.done
    }

    /// What the effect has told the host so far.
    pub fn events(&self) -> Events {
        self.host
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .events
            .clone()
    }

    /// The background intensity the effect set (128 is unchanged).
    pub fn intensity(&self) -> u8 {
        self.host
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .intensity
    }

    /// Moves the units (they step and turn while the effect plays).
    pub fn set_units(&mut self, units: &[Unit]) {
        let mut host = self
            .host
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        host.units = units.to_vec();
    }

    /// One frame seen through `camera`. `None` once the effect has ended
    /// (or faulted).
    pub fn step(&mut self, camera: &PsxCamera) -> Option<RenderList> {
        let prims = self.advance(camera)?;
        let mut host = self
            .host
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // `PSXTextureMgr.CaptureBG` runs as the frame renders: the key is
        // set before the frame's packets pick their textures.
        let capture = host.background.take();
        if let Some(params) = capture {
            let key = (params[1] >> 8 << 4) | (params[0] >> 6);
            host.vram.background = Some(key as u16);
        }
        let mut list = render_list(self.effect, &prims, &mut host.vram);
        list.background_capture = capture;
        Some(list)
    }

    /// What the effect has done to each unit's model so far, by `btl_id`
    /// (only the units it touched).
    pub fn changes(&self) -> std::collections::BTreeMap<u16, UnitChange> {
        self.host
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .changes
            .iter()
            .filter(|(_, change)| !change.is_empty())
            .map(|(id, change)| {
                let mut change = change.clone();
                if let Some(motion) = change.motion.as_mut() {
                    motion.now = self.frame;
                }
                (*id, change)
            })
            .collect()
    }

    /// One frame without building its render list (for the effect's timing
    /// only). `None` once the effect has ended (or faulted).
    pub fn advance(&mut self, camera: &PsxCamera) -> Option<Vec<crate::plugin::Prim>> {
        if self.done {
            return None;
        }
        let prims = guarded(|| {
            let (matrix, distance) = camera_bytes(camera);
            let _ = self.plugin.m.write_bytes(CAMERA_MATRIX, &matrix);
            let _ = self.plugin.m.write_bytes(CAMERA_DISTANCE, &distance);
            self.plugin.frame()
        })
        .flatten();
        let Some(prims) = prims else {
            self.done = true;
            return None;
        };
        self.frame += 1;
        self.host
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .frame = self.frame;
        Some(prims)
    }

    /// Every `LoadImage` so far.
    pub fn uploads(&self) -> Vec<crate::loader::Upload> {
        self.host
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .uploads
            .clone()
    }

    pub fn vram(&self) -> Vram {
        self.host
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .vram
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat block of bytes standing for the plugin's memory.
    struct Block(Vec<u8>);

    impl Access for Block {
        fn read(&self, at: u64, len: usize) -> Option<Vec<u8>> {
            self.0
                .get(at as usize..at as usize + len)
                .map(<[u8]>::to_vec)
        }

        fn write(&mut self, at: u64, bytes: &[u8]) {
            self.0[at as usize..at as usize + bytes.len()].copy_from_slice(bytes);
        }
    }

    fn host() -> BattleHost {
        let unit = Unit {
            btl_id: 1,
            player: true,
            slot: 0,
            serial: 0,
            position: [100, 0, -200],
            base: [100, 0, -200],
            yaw: 1024,
            height: 500,
            radius: 256,
            geo_radius: 256,
            tar_bone: 0,
            bones: Default::default(),
            motion_frames: Default::default(),
        };
        BattleHost {
            units: vec![unit],
            ..Default::default()
        }
    }

    fn code(command: u8, btl_id: u8) -> i32 {
        (i32::from(command) << 24) | i32::from(btl_id)
    }

    /// Callback 9 answers the current motion's frame count
    /// (`GeoAnim.geoAnimGetNumFrames`, at least 1), Freya's jump clips for
    /// the chant motions while Jump plays.
    #[test]
    fn frame_count_follows_the_motion_the_effect_set() {
        let mut host = host();
        let mut frames = vec![0u16; JUMP_MOTIONS + 3];
        frames[9] = 21;
        frames[27] = 13;
        frames[JUMP_MOTIONS + 1] = 40;
        host.units[0].player = true;
        host.units[0].motion_frames = std::sync::Arc::new(frames);
        let mut memory = Block(vec![0; 64]);
        // Before the effect sets one: `MP_IDLE_CMD`.
        assert_eq!(host.answer(&mut memory, code(9, 1), [0; 4], 0), 21);
        host.answer(&mut memory, code(12, 1), [27, 0, 0, 0], 0);
        assert_eq!(host.answer(&mut memory, code(9, 1), [0; 4], 0), 13);
        // A clip not known answers 1.
        host.answer(&mut memory, code(12, 1), [26, 0, 0, 0], 0);
        assert_eq!(host.answer(&mut memory, code(9, 1), [0; 4], 0), 1);
        host.effect = 393;
        host.answer(&mut memory, code(12, 1), [27, 0, 0, 0], 0);
        assert_eq!(host.answer(&mut memory, code(9, 1), [0; 4], 0), 40);
    }

    #[test]
    fn set_commands_stage_the_unit_and_later_gets_see_it() {
        let mut host = host();
        let mut memory = Block(vec![0; 64]);
        // Set Position (-50, -120, 30): later gets answer with it, and
        // the bones follow.
        for (at, value) in [-50i16, -120, 30].into_iter().enumerate() {
            memory.write(8 + at as u64 * 2, &value.to_le_bytes());
        }
        host.answer(&mut memory, code(2, 1), [0; 4], 8);
        let mut read = Block(vec![0; 64]);
        host.answer(&mut read, code(1, 1), [0; 4], 16);
        assert_eq!(read.0[16..22], memory.0[8..14]);
        // Set Size, 2.0 on x: Get Size answers 8192.
        host.answer(&mut memory, code(6, 1), [1 | 128, 8192, 4096, 4096], 0);
        let mut size = Block(vec![0; 64]);
        host.answer(&mut size, code(5, 1), [0; 4], 8);
        assert_eq!(i32::from_le_bytes(size.0[8..12].try_into().unwrap()), 8192);
        // Hide, then show: the priority rule keeps a Jump's hide.
        host.effect = 393;
        host.answer(&mut memory, code(18, 1), [1, 0, 0, 0], 0);
        assert_eq!(host.answer(&mut memory, code(17, 1), [0; 4], 0), 1);
        host.answer(&mut memory, code(18, 1), [0, 0, 0, 0], 0);
        assert_eq!(host.answer(&mut memory, code(17, 1), [0; 4], 0), 0);
        // Hide all meshes, show all.
        host.answer(&mut memory, code(25, 1), [0, 0, 0, 0], 0);
        assert!(host.changes[&1].meshes_hidden);
        host.answer(&mut memory, code(25, 1), [2, 0, 0, 0], 0);
        assert!(!host.changes[&1].meshes_hidden);
        host.answer(&mut memory, code(25, 1), [1, 0, 0, 0], 0);
        assert!(host.changes[&1].vanished && !host.changes[&1].meshes_hidden);
        host.answer(&mut memory, code(25, 1), [2, 0, 0, 0], 0);
        assert!(!host.changes[&1].vanished);
        // Set Angles to a value: Euler (0, v - 180, 180 + v).
        let mut angles = Block(vec![0; 64]);
        angles.write(8, &1024i16.to_le_bytes());
        angles.write(10, &2048i16.to_le_bytes());
        angles.write(12, &0i16.to_le_bytes());
        host.answer(&mut angles, code(4, 1), [2, 0, 0, 0], 8);
        let euler = host.changes[&1].euler.expect("set");
        assert_eq!(euler, [90.0, 0.0, 180.0]);
    }

    #[test]
    fn an_effect_motion_holds_while_stopped() {
        // Jump: set at frame 0, stopped from frame 5, running again from frame 12.
        let mut motion = EffectMotion {
            effect: 393,
            index: 26,
            set: 0,
            stops: vec![(5, true), (12, false)],
            now: 4,
        };
        assert_eq!(motion.frames(), 4);
        motion.now = 12;
        assert_eq!(motion.frames(), 5);
        motion.now = 15;
        assert_eq!(motion.frames(), 8);
    }

    #[test]
    fn a_background_capture_is_kept_for_the_frame() {
        let mut host = host();
        let mut memory = Block(vec![0; 64]);
        for (i, value) in [448i32, 256, 128, 160, 0, 360, 3950]
            .into_iter()
            .enumerate()
        {
            memory.write(8 + i as u64 * 4, &value.to_le_bytes());
        }
        assert_eq!(host.background, None);
        host.answer(&mut memory, code(120, 0), [0; 4], 8);
        assert_eq!(host.background, Some([448, 256, 128, 160, 0, 360, 3950]));
    }
}
