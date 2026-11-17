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

/// One battle unit as the plugin sees it (PSX battle space: y down).
#[derive(Debug, Clone, Copy, PartialEq)]
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
}

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
    /// Callback 32: sound commands (`arg0`, `arg1`).
    pub sounds: Vec<(u32, i32, i32)>,
}

/// `SFX.BattleCallback` over [`Setup::units`].
#[derive(Debug, Default)]
pub struct BattleHost {
    pub vram: Vram,
    pub units: Vec<Unit>,
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
                self.events.sounds.push((self.frame, a0, a1));
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
        let Some(unit) = self.unit(btl_id).copied() else {
            // `BattleCallbackWithBtl`: no such unit.
            return i32::from(command == 9);
        };
        let middle = [
            unit.position[0],
            unit.position[1].saturating_sub(unit.height / 2),
            unit.position[2],
        ];
        match command {
            1 if p != 0 => {
                let at = if a0 & 1 == 0 {
                    unit.position
                } else {
                    unit.base
                };
                if a0 >= 2 {
                    write_i16(memory, p, &[at[0]]);
                    write_i16(memory, p + 4, &[at[2]]);
                } else {
                    write_i16(memory, p, &at);
                }
                0
            }
            3 if p != 0 => {
                let angles = match a0 {
                    0 => [0, 0, 0],
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
            5 if p != 0 => {
                let bytes: Vec<u8> = [4096i32; 3].iter().flat_map(|v| v.to_le_bytes()).collect();
                memory.write(p, &bytes);
                0
            }
            9 => 1,
            14 => {
                let at = if a1 == 0 { unit.position } else { middle };
                match a0 {
                    0 if p != 0 => write_i16(memory, p, &at),
                    1 => return i32::from(at[1]),
                    2 if p != 0 => {
                        // `PSXMAT` of the identity transform: r, pad, t.
                        let mut bytes = Vec::with_capacity(32);
                        for value in [-4096i16, 0, 0, 0, 4096, 0, 0, 0, 4096, 0] {
                            bytes.extend_from_slice(&value.to_le_bytes());
                        }
                        for value in at {
                            bytes.extend_from_slice(&i32::from(value).to_le_bytes());
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
        Some(render_list(self.effect, &prims, &mut host.vram))
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
