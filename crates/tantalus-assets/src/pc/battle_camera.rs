//! Battle cameras: the camera block of a scene's `.raw17` (`camOffset`) and
//! the special-effect plugin's camera script that plays it.
//!
//! Sources:
//! - `Memoria/Battle/SFX/SFXDataCamera.cs` `LoadFromBSC`: the block is an
//!   offset table of camera sets. 0 opens the battle, 1 is the default
//!   camera, 2 the victory, 3 and 4 the reactions to a party member's and an
//!   enemy's effect, 5 and up one per enemy slot.
//! - The rewritten `FF9SpecialEffectPlugin`:
//!   `u0003_SfxSelectBlockVariant.cs` (sets, variants and the selector),
//!   `u0113_SFX_UpdateCamera_0.cs` (`CamScriptTick`, the per-frame script),
//!   `u0009_EvalOrbitTrack.cs` (the eye and target tracks),
//!   `u0008_sub_1800148F0.cs` (`GetAnchorTransform`, `ComputeOrbitPosition`),
//!   `u0112_CamLookAtMatrix.cs` (key push, wave, shake, look-at matrix),
//!   `u0114_SFX_MoveFreeCamera_0.cs` (`SFX_StartPlungeCamera`,
//!   `SfxResetAndSelectBlock`) and `u0116_SFX_SendIntData_0.cs`
//!   (`SetEnemyCamera`).
//!
//! Positions are PSX battle space (y down). The host side of the plugin
//! (unit positions, angles and heights) is a [`CameraWorld`] the caller
//! fills. Not ported: the per-character key table at `0x18004AC60` (keys
//! with the top bits set fall back to their 7-bit fields), the host
//! "slave" query, anchor 9's stored point and anchor 15's object.

use std::f64::consts::PI;

/// One unit as the camera sees it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CameraActor {
    /// Current position, PSX space (`x`, `-y`, `z`).
    pub position: [i16; 3],
    /// Battle (base) position, read by anchors with bit 0x20.
    pub base: [i16; 3],
    /// Facing: the Unity `eulerAngles.y` in 4096ths of a turn.
    pub yaw: i16,
    pub player: bool,
    /// `geo_radius`, added to orbit distances.
    pub radius: i32,
    /// `geo_height` (players) or the model's height (enemies).
    pub height: i32,
}

impl CameraActor {
    /// `HostQueryObjectVector`: the low, middle and high points
    /// (`y - h/4`, `y - h/2`, `y - 3h/4`).
    fn heights(&self) -> [i16; 3] {
        let y = i32::from(self.position[1]);
        let h = self.height;
        [
            (y - (h >> 2)) as i16,
            (y - (h >> 1)) as i16,
            (y + (h >> 2) - h) as i16,
        ]
    }
}

/// What the plugin asks the host about.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CameraWorld {
    /// Party first, then enemies (`BattleObjects`).
    pub actors: Vec<CameraActor>,
    /// `PatAddr[PatNum].Monster[0]` x and z (anchor 11).
    pub put: [i16; 2],
    /// `btl_scene.Info.StartType` (2: back attack).
    pub start_type: u8,
}

/// The camera after a frame: `SFX_UpdateCamera`'s 13 floats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PsxCamera {
    /// Rotation rows, 4096 = 1.
    pub rotation: [[i32; 3]; 3],
    pub translation: [i32; 3],
    /// Projection distance (`CameraProjectionDistance`).
    pub h: i32,
}

/// A camera block: the bytes from `camOffset` to the end of the `.raw17`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CameraBlock {
    pub bytes: Vec<u8>,
}

impl CameraBlock {
    /// The block of a `.raw17` file (header word 1 is `camOffset`).
    pub fn from_raw17(raw: &[u8]) -> Option<Self> {
        let offset = usize::try_from(i16::from_le_bytes([*raw.get(2)?, *raw.get(3)?])).ok()?;
        (offset > 0 && offset < raw.len()).then(|| Self {
            bytes: raw[offset..].to_vec(),
        })
    }

    fn u16_at(&self, at: usize) -> Option<u16> {
        Some(u16::from_le_bytes([
            *self.bytes.get(at)?,
            *self.bytes.get(at + 1)?,
        ]))
    }

    /// Number of camera sets (the first offset over 2).
    pub fn sets(&self) -> usize {
        self.u16_at(0).map_or(0, |first| usize::from(first) / 2)
    }

    /// Byte offset of set `index`.
    pub fn set(&self, index: usize) -> Option<usize> {
        (index < self.sets())
            .then(|| self.u16_at(2 * index))
            .flatten()
            .map(usize::from)
    }
}

/// How `SfxSelectBlockVariant` picks a variant (mode 2 asks the host).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VariantContext {
    /// `seq_work_set.CameraNo`.
    pub camera_no: u8,
    /// `GetStartType`.
    pub start_type: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Track {
    start: [i16; 4],
    start_anchor: [u8; 2],
    end: [i16; 4],
    end_anchor: [u8; 2],
    state: u8,
    wrap: u8,
    curve: u8,
    hold: u8,
    length: u8,
    remaining: u8,
}

/// The plugin's camera state (`qword_18021FF00` and on).
#[derive(Debug, Clone, PartialEq)]
pub struct CameraScript {
    block: CameraBlock,
    /// Length of the scene block; effect cameras are appended after it.
    scene_len: usize,
    /// Script read position in `block.bytes`.
    cursor: Option<usize>,
    frame: u32,
    tracks: [Track; 2],
    /// `unk_180220060`: custom points 21..24, the saved eye (25) and target
    /// (26); x, y, z each.
    points: [[i16; 3]; 8],
    /// Rotations of points 21..24 (`entry[24..26]`).
    point_rot: [[i16; 3]; 4],
    /// `dword_18021FF0C`.
    speed: i32,
    wave: [i32; 2],
    wave_phase: [i32; 2],
    /// Focal: remaining, duration, flags, start, end.
    focal: [i32; 5],
    /// `dword_1802200BE`: projection distance and its next value.
    pair: [u16; 2],
    /// Shake: flags, start, end, remaining, duration.
    shake: [i32; 5],
    save: u8,
    stop: bool,
    skip_variant: bool,
    /// `byte_1802200BC`.
    started: u8,
    /// `dword_180211E4C`.
    proj_offset: i32,
    /// `dword_180220108` and `dword_18021FF1C`.
    variant: u32,
    start_variant: u32,
    selector_bits: u8,
    pub phase: u8,
    rand: u32,
    /// `byte_1802200B3` (offset 83 of the camera block at `0x180220060`):
    /// the random side bit `SfxStateReset` and `SfxStartTrackForEntry`
    /// draw. An effect's alternate camera (`SfxOpStartTrackBlock` mode 3)
    /// plays this entry of its list.
    side: u8,
    /// Units the anchors follow: the effect's caster and target
    /// (`qword_18021FFF0`, `qword_18021FFF8`), the camera caster and
    /// target (`SetCameraTarget`), and its average target point.
    pub caster: Option<usize>,
    pub target: Option<usize>,
    pub camera_caster: Option<usize>,
    pub camera_target: Option<usize>,
    pub target_center: [i16; 3],
    pub context: VariantContext,
    eye: [i16; 4],
    look: [i16; 4],
    matrix: Option<PsxCamera>,
}

const SCALE: f64 = 4096.8;

fn trunc(value: f64) -> i32 {
    value as i32
}

impl CameraScript {
    pub fn new(block: CameraBlock, seed: u32) -> Self {
        Self {
            scene_len: block.bytes.len(),
            block,
            cursor: None,
            frame: 0,
            tracks: [Track::default(); 2],
            points: [[0; 3]; 8],
            point_rot: [[0; 3]; 4],
            speed: 0,
            wave: [0; 2],
            wave_phase: [0; 2],
            focal: [0; 5],
            pair: [0; 2],
            shake: [0; 5],
            save: 0,
            stop: false,
            skip_variant: false,
            started: 0,
            proj_offset: 0,
            variant: 0,
            start_variant: 0,
            selector_bits: 0,
            phase: 0,
            rand: seed,
            side: 0,
            caster: None,
            target: None,
            camera_caster: None,
            camera_target: None,
            target_center: [0; 3],
            context: VariantContext::default(),
            eye: [0; 4],
            look: [0; 4],
            matrix: None,
        }
    }

    /// Whether a script is still running.
    pub fn running(&self) -> bool {
        self.cursor.is_some()
    }

    /// The eye and target points of the last frame (PSX space).
    pub fn eye_and_target(&self) -> ([i16; 3], [i16; 3]) {
        (
            [self.eye[0], self.eye[1], self.eye[2]],
            [self.look[0], self.look[1], self.look[2]],
        )
    }

    /// `SfxStateReset` (`u0003_SfxSelectBlockVariant.cs`) at an effect's
    /// start: draws the side bit, `(PsxRandStep() >> 16) & 1`.
    pub fn draw_side(&mut self) -> u8 {
        self.side = ((self.rand() >> 16) & 1) as u8;
        self.side
    }

    /// The side bit last drawn.
    pub fn side(&self) -> u8 {
        self.side
    }

    fn rand(&mut self) -> u32 {
        self.rand = self.rand.wrapping_mul(1_103_515_245).wrapping_add(12345);
        self.rand
    }

    fn u16_at(&self, at: usize) -> u16 {
        self.block.u16_at(at).unwrap_or(0)
    }

    fn byte(&self, at: usize) -> u8 {
        self.block.bytes.get(at).copied().unwrap_or(0)
    }

    /// Clears the `0x68` bytes at `qword_18021FF00` (script, timers,
    /// focal, shake, wave, save).
    fn clear_state(&mut self) {
        self.cursor = None;
        self.frame = 0;
        self.speed = 0;
        self.wave = [0; 2];
        self.wave_phase = [0; 2];
        self.focal = [0; 5];
        self.shake = [0; 5];
        self.save = 0;
        self.skip_variant = false;
        self.start_variant = 0;
    }

    /// `SFX_StartPlungeCamera`: the opening (set 0).
    pub fn start_plunge(&mut self, proj_offset: i32) {
        self.reset_and_select(self.block.set(0), proj_offset);
        self.caster = None;
        self.target = None;
    }

    /// `SetCamera(set, arg)` (`SfxResetAndSelectBlock`): set 2 is the
    /// victory.
    pub fn set_camera(&mut self, set: usize, arg: i32) {
        self.reset_and_select(self.block.set(set), arg);
    }

    fn reset_and_select(&mut self, set: Option<usize>, proj_offset: i32) {
        self.proj_offset = proj_offset;
        self.stop = false;
        self.started = 0;
        self.clear_state();
        self.caster = None;
        self.target = None;
        match set {
            Some(set) => self.select_variant(set),
            None => self.cursor = None,
        }
    }

    /// `SfxStartTrackBlock`: an effect's reaction camera (set 3 for a
    /// player's effect, 4 for an enemy's, 5 + slot an enemy's own) with
    /// `caster` and `target` as anchors 1 and 5.
    pub fn start_track_block(
        &mut self,
        set: usize,
        proj_offset: i32,
        caster: usize,
        target: Option<usize>,
        world: &CameraWorld,
    ) {
        let Some(set) = self.block.set(set) else {
            return;
        };
        // `SfxStateReset`.
        self.proj_offset = 0;
        self.clear_state();
        self.caster = Some(caster);
        self.target = target;
        self.proj_offset = proj_offset;
        if self.started & 2 != 0 {
            self.started = 0;
        }
        self.stop = false;
        self.speed = 0;
        for index in 0..2 {
            self.snap_running(index, world);
        }
        self.select_variant(set);
    }

    /// `SfxStartTrackBlock` on a camera set outside the scene block: an
    /// effect's own camera (`PLAY_CAMERA`), `set` the bytes of its small
    /// file. The bytes join the block once (offsets in a set are relative
    /// to the set).
    pub fn start_effect_camera(
        &mut self,
        set: &[u8],
        proj_offset: i32,
        caster: usize,
        target: Option<usize>,
        world: &CameraWorld,
    ) {
        if set.len() < 4 {
            return;
        }
        let at = match self
            .block
            .bytes
            .windows(set.len())
            .skip(self.scene_len)
            .position(|window| window == set)
        {
            Some(position) => self.scene_len + position,
            None => {
                let at = self.block.bytes.len();
                self.block.bytes.extend_from_slice(set);
                at
            }
        };
        self.proj_offset = 0;
        self.clear_state();
        self.caster = Some(caster);
        self.target = target;
        self.proj_offset = proj_offset;
        if self.started & 2 != 0 {
            self.started = 0;
        }
        self.stop = false;
        self.speed = 0;
        for index in 0..2 {
            self.snap_running(index, world);
        }
        self.select_variant(at);
    }

    /// `SetCameraTarget` then `SetEnemyCamera` (`SfxStartTrackForEntry`):
    /// enemy `slot`'s camera (set 5 + slot) with `camera_no` as its variant.
    pub fn set_enemy_camera(
        &mut self,
        slot: usize,
        caster: usize,
        target: Option<usize>,
        center: [i16; 3],
        camera_no: u8,
        world: &CameraWorld,
    ) {
        // `SfxStartTrackForEntry` draws the side bit first.
        self.draw_side();
        self.camera_caster = Some(caster);
        self.camera_target = target;
        self.target_center = center;
        self.context.camera_no = camera_no;
        let Some(set) = self.block.set(5 + slot) else {
            return;
        };
        self.clear_state();
        self.proj_offset = 0;
        if self.started & 2 != 0 {
            self.started = 0;
        }
        self.stop = false;
        for index in 0..2 {
            self.snap_running(index, world);
        }
        self.select_variant(set);
        self.started = 0;
    }

    /// `SfxBeginBlock`: back to the default camera (set 1).
    pub fn begin_default(&mut self) {
        self.proj_offset = 0;
        if let Some(set) = self.block.set(1) {
            if !self.skip_variant {
                self.variant = self.start_variant;
                self.select_variant(set);
                self.pair[0] = self.pair[1];
            }
            self.started = 1;
            self.skip_variant = false;
        }
        self.caster = None;
        self.target = None;
    }

    fn snap_running(&mut self, index: usize, world: &CameraWorld) {
        let track = self.tracks[index];
        if track.remaining != 0 && matches!(track.state, 2 | 3) {
            let end = self.eval_track(true, index, world);
            self.tracks[index].end = end;
            self.tracks[index].state = 1;
        }
    }

    /// `SfxSelectBlockVariant`.
    fn select_variant(&mut self, set: usize) {
        self.frame = 1;
        let header = self.u16_at(set) as i16 as i32;
        let mut cursor = set + 2;
        let mut variants = [None; 4];
        let mut flags = header;
        for variant in &mut variants {
            if flags & 1 != 0 {
                *variant = Some(set + usize::from(self.u16_at(cursor)));
                cursor += 2;
            }
            flags >>= 1;
        }
        let mut records = set + usize::from(self.u16_at(cursor));
        for k in 0..4 {
            if flags & 1 != 0 {
                for axis in 0..3 {
                    self.points[k][axis] = self.u16_at(records + 2 * axis) as i16;
                }
                records += 6;
            } else {
                self.points[k] = [0; 3];
            }
            self.point_rot[k] = [0; 3];
            flags >>= 1;
        }
        let selected = self.select_index(variants[3]);
        self.cursor = if selected < 4 {
            variants[selected as usize]
        } else {
            None
        };
        if header & 0x200 != 0 {
            self.selector_bits = ((header >> (2 * selected + 10)) & 3) as u8;
        }
    }

    /// `SelectVariantIndex`.
    fn select_index(&mut self, selector: Option<usize>) -> u32 {
        let Some(selector) = selector else {
            return self.variant;
        };
        let word = u32::from(self.u16_at(selector));
        let result = match word >> 14 {
            0 => (word >> 12) & 3,
            1 => {
                let roll = (self.rand() >> 16) as i32 % 9;
                let threshold = ((word >> 10) & 3) as i32;
                if word & 0x3000 == 0 {
                    u32::from(roll >= threshold + 5)
                } else if roll >= threshold + 3 {
                    u32::from(roll >= threshold + 6) + 1
                } else {
                    0
                }
            }
            3 => match self.selector_bits & 3 {
                2 => 0,
                3 => 2,
                bits => u32::from(bits),
            },
            _ => match (word >> 12) & 3 {
                0 => {
                    if self.context.start_type == 2 {
                        self.start_variant = 2;
                        2
                    } else {
                        self.start_variant = 0;
                        self.rand();
                        0
                    }
                }
                1 => u32::from(self.context.camera_no),
                // The tracked target's size (`state`/`value` at +56/+60)
                // and the owner's vote are not modelled.
                _ => 0,
            },
        };
        self.variant = result;
        result
    }

    /// One battle frame (`SFX_UpdateCamera`): runs the script and returns
    /// the camera, or `None` before both tracks have a key.
    pub fn update(&mut self, world: &CameraWorld) -> Option<PsxCamera> {
        if self.tick(world) {
            self.tick(world);
        }
        self.matrix
    }

    /// `CamScriptTick`; true when a block ended.
    fn tick(&mut self, world: &CameraWorld) -> bool {
        if self.frame != 0 {
            if let Some(mut p) = self.cursor {
                let mut commands = 0;
                while p + 4 <= self.block.bytes.len()
                    && self.frame == u32::from(self.u16_at(p) & 0x3FF)
                {
                    if self.started & 1 == 0 {
                        self.pair[0] = 512;
                        self.started |= 1;
                    }
                    let flags =
                        (u32::from(self.u16_at(p) >> 10) << 16) | u32::from(self.u16_at(p + 2));
                    p += 4;
                    for (index, shift) in [(0usize, 0u32), (1, 3)] {
                        let kind = ((flags >> shift) & 7) as u8;
                        let track = self.tracks[index];
                        if matches!(kind, 2 | 3)
                            && matches!(track.state, 2 | 3)
                            && track.remaining != 0
                        {
                            let end = self.eval_track(true, index, world);
                            self.tracks[index].end = end;
                        }
                        if kind != 0 && self.tracks[index].hold == 0 {
                            self.tracks[index].state = kind;
                        }
                        if (1..=3).contains(&kind) {
                            self.push_key(index, p);
                            p += 6;
                        }
                        if (2..=4).contains(&kind) {
                            if self.tracks[index].hold == 0 {
                                let (length, curve, wrap) =
                                    (self.byte(p), self.byte(p + 2), self.byte(p + 3));
                                let track = &mut self.tracks[index];
                                track.remaining = length;
                                track.length = length;
                                track.curve = curve;
                                track.wrap = wrap;
                            }
                            if kind == 4 {
                                self.tracks[index].end[2] = i16::from(self.byte(p + 1) as i8) << 5;
                            }
                            p += 4;
                        }
                        self.tracks[index].hold = 0;
                    }
                    if flags & 0x40 != 0 {
                        self.speed = i32::from(self.byte(p));
                        p += 2;
                    }
                    if flags & 0x80 != 0 {
                        self.save = 1;
                    }
                    if flags & 0x10_0100 != 0 {
                        self.stop = true;
                    }
                    if flags & 0x200 != 0 {
                        self.wave[0] = i32::from(self.byte(p));
                        p += 2;
                    }
                    if flags & 0x400 != 0 {
                        self.wave[1] = i32::from(self.byte(p));
                        p += 2;
                    }
                    if flags & 0x800 != 0 {
                        let duration = i32::from(self.byte(p));
                        let end = self.u16_at(p + 2);
                        self.focal = [
                            duration,
                            duration,
                            i32::from(self.byte(p + 1)),
                            i32::from(self.pair[0]),
                            i32::from(if end != 0 { end } else { self.pair[1] }),
                        ];
                        p += 4;
                    }
                    if flags & 0x1000 != 0 {
                        self.shake = [
                            i32::from(self.byte(p + 3) & 3),
                            i32::from(self.byte(p)),
                            i32::from(self.byte(p + 1)),
                            i32::from(self.byte(p + 2)),
                            i32::from(self.byte(p + 2)),
                        ];
                        p += 4;
                    }
                    if flags & 0x4000 != 0 {
                        if self.byte(p) == 1 {
                            self.phase = 1;
                        }
                        p += 2;
                    }
                    if flags & 0x8000 != 0 {
                        self.cursor = Some(p);
                        return self.end_block(p, world);
                    }
                    if flags & 0x8_0000 != 0 {
                        self.skip_variant = true;
                    }
                    commands += 1;
                    if commands >= 10 {
                        break;
                    }
                }
                self.cursor = (self.u16_at(p) != 0).then_some(p);
            }
            for track in &mut self.tracks {
                track.remaining = track.remaining.saturating_sub(1);
            }
        }
        if self.tracks[0].state != 0 && self.tracks[1].state != 0 {
            let mut eye = self.eval_track(false, 0, world);
            let mut look = self.eval_track(false, 1, world);
            if self.save == 1 {
                self.save = 2;
                self.points[4] = [eye[0], eye[1], eye[2]];
                self.points[5] = [look[0], look[1], look[2]];
            }
            self.apply_wave(&mut eye, &mut look);
            if self.shake[3] != 0 {
                self.apply_shake(&mut look);
            }
            self.eye = eye;
            self.look = look;
            self.matrix = Some(self.look_at(eye, look));
        }
        if self.focal[0] != 0 {
            self.focal[0] -= 1;
            let t = f64::from(self.focal[0]) / f64::from(self.focal[1]);
            let value =
                trunc((1.0 - t) * f64::from(self.focal[4]) + f64::from(self.focal[3]) * t) as u16;
            if self.focal[2] & 1 != 0 {
                self.pair[1] = value;
            }
            if self.focal[2] & 2 != 0 {
                self.pair[0] = value;
            }
        }
        if let Some(matrix) = &mut self.matrix {
            matrix.h = i32::from(self.pair[0]);
        }
        if self.frame <= 0x3FE && !self.stop {
            self.frame += 1;
        }
        false
    }

    /// `CamScriptTickEndBlock`: freezes running tracks, takes the next
    /// timings from the word after the command and continues with the
    /// default camera (set 1).
    fn end_block(&mut self, p: usize, world: &CameraWorld) -> bool {
        for index in 0..2 {
            self.snap_running(index, world);
        }
        let state = (((self.byte(p) as i8) >> 5) & 3) as u8;
        let length = self.byte(p + 1);
        let curve = self.byte(p + 2);
        for track in &mut self.tracks {
            track.hold = 1;
            track.state = state;
            track.remaining = length;
            track.length = length;
            track.curve = curve;
            track.wrap = 0;
        }
        if let Some(set) = self.block.set(1) {
            self.variant = self.start_variant;
            self.select_variant(set);
            self.pair[0] = self.pair[1];
        }
        true
    }

    /// `CamPushTrackKey`: the end key becomes the start, the 6-byte key at
    /// `at` the new end.
    fn push_key(&mut self, index: usize, at: usize) {
        let packed = self.u16_at(at + 2);
        let second = self.u16_at(at + 4);
        let anchor = [self.byte(at), self.byte(at + 1)];
        let track = &mut self.tracks[index];
        track.start = track.end;
        track.start_anchor = track.end_anchor;
        track.end_anchor = anchor;
        track.end = [
            ((packed & 0x7F) << 5) as i16,
            (((packed >> 7) & 0x7F) << 5) as i16,
            ((second & 0x7F) << 5) as i16,
            (((second >> 7) & 0x7F) << 7) as i16,
        ];
    }

    /// `EvalOrbitTrack`: `snap` gives the track's current key in its own
    /// terms (to freeze it), otherwise the world point and roll.
    fn eval_track(&mut self, snap: bool, index: usize, world: &CameraWorld) -> [i16; 4] {
        let mut track = self.tracks[index];
        if matches!(track.state, 2 | 3) && track.remaining == 0 {
            track.state = 1;
        }
        let mut weights = [4096i32, 0];
        let mut complements = [0i32; 2];
        if track.state > 1 {
            let mut modes = i32::from(track.curve);
            let length = i32::from(track.length);
            for i in 0..2 {
                let elapsed = i32::from(track.remaining);
                if length != 0 && elapsed != 0 {
                    let factor = ((4096 - length * self.speed) as u32).wrapping_shl(17)
                        / (length * length) as u32;
                    match modes & 3 {
                        0 => weights[i] = (elapsed * (0x1000_0000 / length)) >> 16,
                        1 => {
                            let rest = length - elapsed;
                            weights[i] = 4096
                                - (((rest * rest) as u32).wrapping_mul(factor) as i32 >> 17)
                                - self.speed * rest;
                        }
                        2 => {
                            weights[i] = elapsed * self.speed
                                + (((elapsed * elapsed) as u32).wrapping_mul(factor) as i32 >> 17);
                        }
                        _ => {}
                    }
                } else {
                    weights[i] = 0;
                }
                complements[i] = 4096 - weights[i];
                modes >>= 2;
            }
        }
        let (p, q) = (weights[0], complements[0]);
        let mut v = [
            track.start[0],
            track.start[1],
            track.start[2],
            track.start[3],
            track.end[0],
            track.end[1],
            track.end[2],
            track.end[3],
        ];
        if track.state > 1 {
            wrap_angles(&mut v, track.wrap);
        }
        let average = average12([v[0], v[1], v[2]], [v[4], v[5], v[6]], p, q);
        let mut avg = [average[0], average[1], average[2], 0];
        let out = match track.state {
            2 => {
                let start = i32::from(v[3]);
                let distance = start + ((sat16(i32::from(v[7]) - start) * complements[1]) >> 12);
                avg[3] = distance as i16;
                if snap {
                    avg
                } else {
                    let mut out = self.orbit(track.end_anchor, &mut avg, world);
                    out[3] = avg[2];
                    if track.remaining == 0 {
                        track.state = 1;
                    }
                    out
                }
            }
            3 => {
                let mut first = [v[0], v[1], v[2], v[3]];
                let mut second = [v[4], v[5], v[6], v[7]];
                let a = self.orbit(track.start_anchor, &mut first, world);
                let b = self.orbit(track.end_anchor, &mut second, world);
                let blend = average12([a[0], a[1], a[2]], [b[0], b[1], b[2]], p, q);
                if snap {
                    let anchor = track.end_anchor;
                    let (mut center, _, _) = self.anchor(anchor[0], world);
                    if anchor[1] & 0x1F != 0 {
                        let (other, _, _) =
                            self.anchor((anchor[1] & 0x3F) | (anchor[0] & 0xC0), world);
                        for axis in 0..3 {
                            center[axis] = (center[axis].wrapping_add(other[axis])) >> 1;
                        }
                    }
                    let dx = i32::from(blend[0]) - i32::from(center[0]);
                    let dy = i32::from(blend[1]) - i32::from(center[1]);
                    let dz = i32::from(blend[2]) - i32::from(center[2]);
                    let horizontal2 = dx * dx + dz * dz;
                    let yaw = trunc(-f64::from(dz).atan2(f64::from(dx)) / (2.0 * PI) * -4096.0);
                    let horizontal = f64::from(trunc(f64::from(horizontal2).sqrt()));
                    let pitch = horizontal.atan2(f64::from(dy));
                    let pitch_units = trunc(-pitch / (2.0 * PI) * -4096.0);
                    let distance = trunc(f64::from(dy * dy + horizontal2).sqrt());
                    track.end_anchor[1] |= 0xC0;
                    [
                        ((1024 - pitch_units) & 0xFFF) as i16,
                        ((-1024 - yaw) & 0xFFF) as i16,
                        avg[2],
                        distance as i16,
                    ]
                } else {
                    if track.remaining == 0 {
                        track.state = 1;
                    }
                    [blend[0], blend[1], blend[2], avg[2]]
                }
            }
            4 => {
                let s = p >> 4;
                let t = q >> 4;
                let basis = [
                    (s * s * s) >> 12,
                    (6 * t * s * s) >> 12,
                    (6 * s * t * t) >> 12,
                    (t * t * t) >> 12,
                ];
                let mut out = [0i16, 0, 0, avg[2]];
                for (k, w) in basis.iter().enumerate() {
                    for (cell, point) in out.iter_mut().zip(self.points[k]) {
                        *cell = cell.wrapping_add(((w * i32::from(point)) >> 12) as i16);
                    }
                }
                out
            }
            _ => {
                let mut end = [v[4], v[5], v[6], v[7]];
                let mut out = self.orbit(track.end_anchor, &mut end, world);
                out[3] = end[2];
                out
            }
        };
        self.tracks[index] = track;
        out
    }

    /// `GetAnchorTransform`: position, rotation and distance bias of
    /// anchor `code` (low 5 bits the id, `0xC0` the height, `0x20` the
    /// base position).
    fn anchor(&self, code: u8, world: &CameraWorld) -> ([i16; 3], [i16; 3], i32) {
        let id = code & 0x1F;
        if id == 0 {
            return ([0; 3], [0; 3], 0);
        }
        if id > 20 {
            let slot = usize::from(id - 21).min(7);
            let rot = self.point_rot.get(slot).copied().unwrap_or([0; 3]);
            return (self.points[slot], rot, 0);
        }
        if id == 11 {
            return ([world.put[0], 0, world.put[1]], [0; 3], 0);
        }
        let object = match id {
            1 => self.caster,
            10 => self.caster.or(Some(0)),
            12 => self.camera_caster,
            _ => None,
        };
        let (actor, rot, heights) = match object.and_then(|index| world.actors.get(index)) {
            Some(actor) => (Some(actor), [0, actor.yaw, 0], actor.heights()),
            None => {
                let index = match id {
                    5 | 9 => self.target,
                    13 | 14 => self.camera_target,
                    _ => None,
                };
                let known = index.and_then(|index| world.actors.get(index));
                let actor = known.or_else(|| world.actors.first());
                let rot = [
                    0,
                    if actor.is_some_and(|a| a.player) {
                        2048
                    } else {
                        0
                    },
                    0,
                ];
                // Without a tracked object the heights stay cleared.
                let heights = known.map_or([0; 3], CameraActor::heights);
                (actor, rot, heights)
            }
        };
        let Some(actor) = actor else {
            return ([0; 3], rot, 0);
        };
        // Bit 0x20 asks the host for the base position, not the current.
        let from = if code & 0x20 != 0 {
            actor.base
        } else {
            actor.position
        };
        let mut position = [from[0], 0, from[2]];
        if id == 14 {
            position[0] = self.target_center[0];
            position[2] = self.target_center[2];
        }
        position[1] = match code & 0xC0 {
            0 => heights[1],
            0x40 => heights[2],
            0xC0 => heights[0],
            _ => 0,
        };
        (position, rot, actor.radius)
    }

    /// `ComputeOrbitPosition`: the point `vec` (pitch, yaw, roll, distance)
    /// away from anchor pair `anchor`; adds the anchor roll to `vec[2]`.
    fn orbit(&self, anchor: [u8; 2], vec: &mut [i16; 4], world: &CameraWorld) -> [i16; 4] {
        let (position, rot, bias) = self.anchor(anchor[0], world);
        let second = anchor[1] as i8;
        let mut center = position;
        if second & 0x1F != 0 {
            let (other, _, _) = self.anchor((anchor[1] & 0x3F) | (anchor[0] & 0xC0), world);
            for axis in 0..3 {
                center[axis] = position[axis].wrapping_add(other[axis]) >> 1;
            }
        }
        let distance = if second >= 0 {
            i32::from(vec[3]) + bias
        } else {
            i32::from(vec[3])
        };
        let angle = |units: i32| f64::from(units) / SCALE * 2.0 * PI;
        let mut out = [0i16; 4];
        if second & 0x40 != 0 {
            let pitch = i32::from(vec[0]);
            let horizontal = (distance * trunc(angle(pitch).cos() * SCALE)) >> 12;
            out[0] = ((horizontal * trunc(angle(i32::from(vec[1])).sin() * -SCALE)) >> 12) as i16;
            out[2] = ((horizontal * trunc(angle(i32::from(vec[1])).cos() * -SCALE)) >> 12) as i16;
            out[1] = ((distance * trunc(angle(pitch).sin() * SCALE)) >> 12) as i16;
        } else {
            let pitch = i32::from(rot[0]) + i32::from(vec[0]);
            let horizontal = (distance * trunc(angle(pitch).cos() * SCALE)) >> 12;
            let mut yaw = i32::from(rot[1]) + i32::from(vec[1]);
            let id = anchor[0] & 0x1F;
            let follows = match id {
                5 | 9 => self.target,
                13 | 14 => self.camera_target,
                _ => None,
            };
            if follows
                .and_then(|index| world.actors.get(index))
                .is_some_and(|actor| actor.player)
            {
                yaw ^= 0xFFF;
            }
            out[0] = ((horizontal * trunc(angle(yaw).sin() * -SCALE)) >> 12) as i16;
            out[2] = ((horizontal * trunc(angle(yaw).cos() * -SCALE)) >> 12) as i16;
            out[1] = ((distance * trunc(angle(pitch).sin() * SCALE)) >> 12) as i16;
            vec[2] = vec[2].wrapping_add(rot[2]);
        }
        for axis in 0..3 {
            out[axis] = out[axis].wrapping_add(center[axis]);
        }
        out
    }

    /// `CamApplyWave`.
    fn apply_wave(&mut self, eye: &mut [i16; 4], look: &mut [i16; 4]) {
        let dx = i32::from(eye[0]) - i32::from(look[0]);
        let dz = i32::from(eye[2]) - i32::from(look[2]);
        let distance = trunc(f64::from(dz * dz + dx * dx).sqrt());
        let mut offset = 0;
        for index in 0..2 {
            let wave = self.wave[index];
            if wave != 0 {
                let phase = self.wave_phase[index];
                let sine = trunc((f64::from(phase << 6) / SCALE * 2.0 * PI).sin() * SCALE);
                offset = ((((sine * distance) >> 13) * (wave & 0xF0)) as u32 >> 11) as i32;
                self.wave_phase[index] = (wave & 0xF) + phase;
                break;
            }
        }
        eye[1] = eye[1].wrapping_add(offset as i16);
        look[1] = look[1].wrapping_add(offset as i16);
    }

    /// `CamShakeTarget`.
    fn apply_shake(&mut self, look: &mut [i16; 4]) {
        self.shake[3] -= 1;
        let t = f64::from(self.shake[3]) / f64::from(self.shake[4].max(1));
        let range = trunc((1.0 - t) * f64::from(self.shake[2]) + f64::from(self.shake[1]) * t) * 2;
        if range == 0 {
            return;
        }
        let flags = self.shake[0];
        if flags & 1 == 0 {
            let seed = self.rand();
            look[1] = look[1].wrapping_add((4 * ((seed >> 16) as i32 % range) - range) as i16);
        }
        if flags & 2 != 0 {
            return;
        }
        let first = self.rand();
        let second = self.rand();
        look[0] = look[0].wrapping_add((4 * ((first >> 16) as i32 % range) - range) as i16);
        look[2] = look[2].wrapping_add((4 * ((second >> 16) as i32 % range) - range) as i16);
    }

    /// `CamLookAtMatrix`: rows right, up, forward from `eye` to `look`,
    /// rolled by `eye[3]`.
    fn look_at(&self, eye: [i16; 4], look: [i16; 4]) -> PsxCamera {
        let d = [
            f64::from(look[0]) - f64::from(eye[0]),
            f64::from(look[1]) - f64::from(eye[1]),
            f64::from(look[2]) - f64::from(eye[2]),
        ];
        let normal = |v: [f64; 3]| {
            let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let scale = if length > 0.0 { SCALE / length } else { 0.0 };
            [
                trunc(v[0] * scale),
                trunc(v[1] * scale),
                trunc(v[2] * scale),
            ]
        };
        let mut forward = normal(d);
        if forward[2] == 0 {
            forward[2] = 1;
        }
        let f = forward.map(f64::from);
        // GTE OP of the forward vector with (0, -4096, 0), then with the
        // result.
        let right = normal([f[2] * 4096.0, 0.0, -f[0] * 4096.0]);
        let r = right.map(f64::from);
        let up = normal([
            f[1] * r[2] - f[2] * r[1],
            f[2] * r[0] - f[0] * r[2],
            f[0] * r[1] - f[1] * r[0],
        ]);
        let rotation = [right, up, forward];
        let eye3 = [i32::from(eye[0]), i32::from(eye[1]), i32::from(eye[2])];
        let apply = |m: &[[i32; 3]; 3], v: [i32; 3]| {
            [0, 1, 2].map(|row| (m[row][0] * v[0] + m[row][1] * v[1] + m[row][2] * v[2]) >> 12)
        };
        let moved = apply(&rotation, eye3);
        let translation = [-moved[0], -moved[1], self.proj_offset - moved[2]];
        // Roll about the view axis (`RotMatrixZ(eye[3])`).
        let roll = f64::from(eye[3]) / 4096.0 * 2.0 * PI;
        let (c, s) = (trunc(roll.cos() * 4096.0), trunc(roll.sin() * 4096.0));
        let rz = [[c, -s, 0], [s, c, 0], [0, 0, 4096]];
        let mut rolled = [[0i32; 3]; 3];
        for (row, out) in rolled.iter_mut().enumerate() {
            for (column, cell) in out.iter_mut().enumerate() {
                *cell = (0..3)
                    .map(|k| rz[row][k] * rotation[k][column])
                    .sum::<i32>()
                    >> 12;
            }
        }
        PsxCamera {
            rotation: rolled,
            translation: apply(&rz, translation),
            h: i32::from(self.pair[0]),
        }
    }
}

fn sat16(value: i32) -> i32 {
    value.clamp(-0x8000, 0x7FFF)
}

/// `GteLoadAverageShort12`: `(a * p + b * q) >> 12` per axis.
fn average12(a: [i16; 3], b: [i16; 3], p: i32, q: i32) -> [i16; 3] {
    [0, 1, 2].map(|axis| {
        let value = (i64::from(a[axis]) * i64::from(p) + i64::from(b[axis]) * i64::from(q)) >> 12;
        sat16(value as i32) as i16
    })
}

/// `EvalOrbitTrackWrapAngles`: moves the start or end angle a turn so the
/// blend goes the intended way round.
fn wrap_angles(v: &mut [i16; 8], modes: u8) {
    let mut modes = modes;
    for k in 0..3 {
        let (start, end) = (v[k], v[k + 4]);
        match modes & 3 {
            0 => {
                if start != end {
                    if (start as u16).wrapping_sub(end as u16) & 0x800 != 0 {
                        if start >= end {
                            v[k + 4] = end.wrapping_add(4096);
                        }
                    } else if start <= end {
                        v[k] = start.wrapping_add(4096);
                    }
                }
            }
            1 if start >= end => v[k + 4] = end.wrapping_add(4096),
            2 if start <= end => v[k] = start.wrapping_add(4096),
            _ => {}
        }
        modes >>= 2;
    }
}

#[cfg(test)]
mod tests;
