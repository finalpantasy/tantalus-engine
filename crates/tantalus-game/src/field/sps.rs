//! Field SPS sprite-strip effects: the state the script ops build and the per-tick animation.
//!
//! `FieldSPSSystem` owns 16 effect slots (`SPSConst.FIELD_DEFAULT_OBJCOUNT`), script ops
//! `SPS` (0xB3) and `SPS2` (0xDA) call `SetObjParm` on one of them
//! (`CommonSPSSystem.SetObjParm`), `EffectUpdate` steps the playback counter every tick and
//! `LateUpdate` draws the slots (the client reads [`SpsSlot::drawn_frame`]). The `.sps` files
//! and their textures are the renderer's: the world only needs each file's frame count, which
//! a provider supplies.

/// `SPSConst.ATTR_*`.
pub const ATTR_VISIBLE: u8 = 1;
pub const ATTR_UPDATE_ANY_FRAME: u8 = 2;
pub const ATTR_UPDATE_THIS_FRAME: u8 = 4;
pub const ATTR_UNLOAD_ON_FINISH: u8 = 8;

/// `SPSConst.FIELD_DEFAULT_OBJCOUNT`.
pub const FIELD_SLOTS: usize = 16;
/// `SPSConst.SCALE_ONE`: a scale of 1.
pub const SCALE_ONE: i32 = 4096;
/// `SPSConst.FRAMERATE_ONE`: the counter runs in 1/16 frames.
pub const FRAMERATE_ONE: i32 = 16;
/// `SPSConst.ABR_OFF`: no PSX blend mode (opaque with a cut-out).
pub const ABR_OFF: u8 = 15;

/// The frame count of `.sps` file `sps` of field `field`, in animation frames (`ushort &
/// 0x7FFF`); `None` when the field has no such effect. Runtime-only provider.
pub type SpsFrameCounts = std::sync::Arc<dyn Fn(i32, i32) -> Option<u16> + Send + Sync>;

/// `SPSEffect`: one effect slot.
#[derive(Debug, Clone, PartialEq)]
pub struct SpsSlot {
    /// `spsBin != null`: a file is loaded.
    pub loaded: bool,
    /// `spsId`: the `.sps` file number, -1 when empty.
    pub id: i32,
    /// The field whose `spt.tcb` and `.sps` files this slot reads (`mapName`): the current
    /// field unless `ChangeFieldOrigin` (op 129) pointed elsewhere at load time.
    pub origin: i32,
    pub attr: u8,
    /// `abr`: 0 to 3 are the PSX blend modes, anything else draws opaque.
    pub abr: u8,
    pub fade: i32,
    /// `charNo` and `boneNo`: the object and bone whose position the effect follows
    /// (op 150); `-1` when it stands free.
    pub char_uid: i32,
    pub bone: i32,
    pub last_frame: i32,
    /// The playback counter in 1/16 frames.
    pub cur_frame: i32,
    /// `frameCount`: the counter's limit (frames times 16).
    pub frame_count: i32,
    pub frame_rate: i32,
    pub duration: i32,
    /// Position in Unity space (`(x, -y, z)` of the script's numbers).
    pub pos: [f32; 3],
    pub scale: i32,
    /// Euler angles in degrees.
    pub rot: [f32; 3],
    pub pos_offset: [f32; 3],
    pub depth_offset: i32,
}

impl Default for SpsSlot {
    /// `SPSEffect.Unload`.
    fn default() -> Self {
        Self {
            loaded: false,
            id: -1,
            origin: 0,
            attr: ATTR_VISIBLE,
            abr: ABR_OFF,
            fade: 128,
            char_uid: -1,
            bone: 0,
            last_frame: -1,
            cur_frame: 0,
            frame_count: 0,
            frame_rate: FRAMERATE_ONE,
            duration: -1,
            pos: [0.0; 3],
            scale: SCALE_ONE,
            rot: [0.0; 3],
            pos_offset: [0.0; 3],
            depth_offset: 0,
        }
    }
}

impl SpsSlot {
    /// The animation frame the slot shows, or `None` while it is not drawn
    /// (`FieldSPSSystem.GenerateSPS`: loaded, visible, and an update attribute set).
    pub fn drawn_frame(&self) -> Option<usize> {
        let drawn = self.loaded
            && self.attr & ATTR_VISIBLE != 0
            && self.attr & (ATTR_UPDATE_ANY_FRAME | ATTR_UPDATE_THIS_FRAME) != 0;
        drawn.then(|| {
            (self.cur_frame >> 4).max(0) as usize % (self.frame_count >> 4).max(1) as usize
        })
    }
}

/// The slots of the current field and the origin `ChangeFieldOrigin` set.
#[derive(Debug, Clone, PartialEq)]
pub struct SpsState {
    pub slots: Vec<SpsSlot>,
    /// `_loadedTCBMapName` as a field id: `None` is the current field. -1 is the world map.
    pub origin: Option<i32>,
}

impl Default for SpsState {
    /// `FieldSPSSystem.Init`: sixteen empty slots.
    fn default() -> Self {
        Self {
            slots: vec![SpsSlot::default(); FIELD_SLOTS],
            origin: None,
        }
    }
}

/// `CommonSPSSystem.AdjustSpsPrecise`: Memoria's corrections of effect positions that sit off
/// their torch, candle or smoke. `pos` is the script position as `(x, -y, z)`; `index` names
/// the slot (`SPS_%04d`).
pub fn adjust_position(field: i32, index: usize, pos: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = pos;
    let at = |wx: f32, wy: f32, wz: f32| x == wx && y == wy && z == wz;
    match (field, index) {
        (2215, 8) if at(1330.0, 1150.0, 1125.0) => [x, 1025.0, z],
        (2301 | 2302, 0) if at(-1200.0, 690.0, 374.0) => [-1330.0, 710.0, 674.0],
        (2301 | 2302, 1) if at(-620.0, 400.0, 504.0) => [x, 350.0, z],
        (2304, 0) if at(-970.0, 1455.0, 1516.0) => [-1030.0, 1400.0, 1716.0],
        (2304, 1) if at(830.0, 1479.0, 1516.0) => [790.0, 1400.0, 2016.0],
        (303, 2) => [4330.0, -1227.0, -700.0],
        (205, 0) if at(-110.0, 850.0, 510.0) => [-122.0, y, z],
        (207, 0) if at(865.0, 465.0, -22.0) => [845.0, 485.0, -82.0],
        (262, 0) if at(355.0, 160.0, 4150.0) => [385.0, y, z],
        (162, 2) if at(-340.0, 660.0, -314.0) => [-350.0, 640.0, -314.0],
        (162, 3) if at(-460.0, 670.0, -314.0) => [-470.0, 650.0, -314.0],
        (162, 4) if at(-570.0, 654.0, -314.0) => [-580.0, 640.0, -314.0],
        (163, 2) if at(821.0, 502.0, 4389.0) => [735.0, 475.0, 4000.0],
        (163, 1) if at(1150.0, 529.0, 4306.0) => [1060.0, 505.0, 4000.0],
        _ => pos,
    }
}

impl super::world::FieldWorld {
    /// `FieldSPSSystem.SetObjParm` (the `SPS` and `SPS2` ops): `object` is the slot,
    /// `operation` one of `SPSConst.OPERATION_*`.
    pub fn sps_op(&mut self, object: i32, operation: i32, arg: [i32; 3]) {
        let Ok(index) = usize::try_from(object) else {
            return;
        };
        // `InitSPSInstance` makes one more slot when the script names the next index.
        if index == self.sps.slots.len() {
            self.sps.slots.push(SpsSlot::default());
        }
        if index >= self.sps.slots.len() {
            return;
        }
        let field = self.field_map_no;
        let [arg0, arg1, arg2] = arg;
        match operation {
            // `OPERATION_CHANGE_FIELD`
            129 => self.sps.origin = Some(if arg0 >= 9000 { -1 } else { arg0 }),
            // `OPERATION_LOAD`
            130 => {
                if arg0 == -1 {
                    // `Unload`: the slot returns to its empty state, placement included.
                    self.sps.slots[index] = SpsSlot::default();
                    return;
                }
                // Prototype effects (`arg1 != 0`: the battle status effects) are not field art.
                if arg1 != 0 {
                    return;
                }
                let origin = self.sps.origin.unwrap_or(field);
                let frames = self
                    .sps_frame_counts
                    .as_ref()
                    .and_then(|provider| provider(origin, arg0));
                let slot = &mut self.sps.slots[index];
                if let Some(frames) = frames {
                    slot.loaded = true;
                    slot.id = arg0;
                    slot.origin = origin;
                    slot.frame_count = i32::from(frames) << 4;
                }
                slot.attr |= ATTR_UPDATE_ANY_FRAME;
                // Memoria: the Wind Shrine interior effects 464, 467, 506 and 510 are dropped.
                if field == 2553 && matches!(slot.id, 464 | 467 | 506 | 510) {
                    slot.loaded = false;
                }
            }
            // `OPERATION_ATTR`. Memoria also keeps the mesh of an effect without a file off after an
            // attribute op on fields 1206, 1223 and 2928 (`if (sps.spsBin != null)`); a slot with no
            // file is never drawn here (`SpsSlot::loaded`), so that tweak changes nothing.
            131 => {
                let slot = &mut self.sps.slots[index];
                if arg1 == 0 {
                    slot.attr &= !(arg0 as u8);
                } else {
                    slot.attr |= arg0 as u8;
                }
            }
            // `OPERATION_POS`
            135 => {
                let slot = &mut self.sps.slots[index];
                // Memoria keeps the Treno and Queen's House position of an empty slot at zero.
                if matches!(field, 911 | 1911) && !slot.loaded {
                    return;
                }
                slot.pos =
                    adjust_position(field, index, [arg0 as f32, -(arg1 as f32), arg2 as f32]);
            }
            // `OPERATION_ROT`
            140 => {
                let degrees = |value: i32| value as f32 / 4096.0 * 360.0;
                self.sps.slots[index].rot = [degrees(arg0), degrees(arg1), degrees(arg2)];
            }
            // `OPERATION_SCALE`
            145 => {
                // Memoria: the candle lights of fields 50 (SPS_0001) and 51 (SPS_0000) are made
                // a bit bigger (`(Int32)(Arg0 * 1.30)`).
                let candle = matches!((field, index), (50, 1) | (51, 0));
                self.sps.slots[index].scale = if candle {
                    (f64::from(arg0) * 1.30) as i32
                } else {
                    arg0
                };
            }
            // `OPERATION_CHAR`: follow bone `arg1` of object `arg0`
            150 => {
                let slot = &mut self.sps.slots[index];
                slot.char_uid = arg0;
                slot.bone = arg1;
            }
            // `OPERATION_FADE`
            155 => self.sps.slots[index].fade = i32::from(arg0 as u8),
            // `OPERATION_ABR`
            156 => self.sps.slots[index].abr = arg0 as u8,
            // `OPERATION_FRAMERATE`
            160 => self.sps.slots[index].frame_rate = arg0,
            // `OPERATION_FRAME`
            161 => self.sps.slots[index].cur_frame = arg0 << 4,
            // `OPERATION_POSOFFSET`
            165 => {
                self.sps.slots[index].pos_offset = [arg0 as f32, -(arg1 as f32), arg2 as f32];
            }
            // `OPERATION_DEPTHOFFSET`
            170 => self.sps.slots[index].depth_offset = arg0,
            _ => {}
        }
    }

    /// One field tick of `FieldSPSSystem`: `GenerateSPS` of the frame before (it runs in
    /// `LateUpdate`, after the scripts) and then `EffectUpdate`.
    pub fn advance_sps(&mut self) {
        for slot in &mut self.sps.slots {
            if slot.loaded
                && slot.attr & ATTR_VISIBLE != 0
                && slot.attr & (ATTR_UPDATE_ANY_FRAME | ATTR_UPDATE_THIS_FRAME) != 0
            {
                slot.attr &= !ATTR_UPDATE_THIS_FRAME;
                slot.last_frame = slot.cur_frame;
            }
        }
        for slot in &mut self.sps.slots {
            if !(slot.loaded && slot.attr & ATTR_VISIBLE != 0 && slot.last_frame != -1) {
                continue;
            }
            slot.last_frame = slot.cur_frame;
            slot.cur_frame += slot.frame_rate;
            if slot.cur_frame >= slot.frame_count {
                slot.cur_frame = 0;
                if slot.attr & ATTR_UNLOAD_ON_FINISH != 0 {
                    *slot = SpsSlot::default();
                }
            } else if slot.cur_frame < 0 {
                slot.cur_frame = ((slot.frame_count >> 4) - 1) << 4;
            }
            if slot.duration > 0 {
                slot.duration -= 1;
            }
            if slot.duration == 0 {
                *slot = SpsSlot::default();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::field::world::FieldWorld;
    use std::sync::Arc;

    fn world() -> FieldWorld {
        let mut world = FieldWorld::new();
        world.field_map_no = 50;
        // Field 50 has effects 42 (14 frames) and 43 (14 frames); nothing else.
        world.sps_frame_counts = Some(Arc::new(|field, id| {
            (field == 50 && matches!(id, 42 | 43)).then_some(14)
        }));
        world
    }

    #[test]
    fn load_places_and_draws_one_frame_per_tick() {
        let mut world = world();
        world.sps_op(0, 130, [42, 0, 0]);
        world.sps_op(0, 135, [100, 200, 300]);
        world.sps_op(0, 145, [2048, 0, 0]);
        let slot = &world.sps.slots[0];
        assert!(slot.loaded && slot.id == 42);
        assert_eq!(slot.pos, [100.0, -200.0, 300.0]);
        assert_eq!(slot.frame_count, 14 * 16);
        assert_eq!(slot.drawn_frame(), Some(0));
        // Frame 0 stays through the first tick (it is first drawn), then 1 per tick.
        world.advance_sps();
        assert_eq!(world.sps.slots[0].drawn_frame(), Some(1));
        world.advance_sps();
        assert_eq!(world.sps.slots[0].drawn_frame(), Some(2));
    }

    #[test]
    fn counter_wraps_and_runs_backwards() {
        let mut world = world();
        world.sps_op(2, 130, [43, 0, 0]);
        world.sps_op(2, 161, [13, 0, 0]);
        world.advance_sps();
        world.advance_sps();
        assert_eq!(
            world.sps.slots[2].cur_frame, 16,
            "wrapped to 0, then stepped"
        );
        world.sps_op(2, 160, [-16, 0, 0]);
        world.advance_sps();
        assert_eq!(world.sps.slots[2].cur_frame, 0);
        world.advance_sps();
        assert_eq!(world.sps.slots[2].cur_frame, 13 << 4);
    }

    #[test]
    fn attributes_hide_unload_and_unknown_files_do_nothing() {
        let mut world = world();
        world.sps_op(0, 130, [42, 0, 0]);
        world.sps_op(0, 131, [1, 0, 0]); // clear VISIBLE
        assert_eq!(world.sps.slots[0].drawn_frame(), None);
        world.sps_op(0, 131, [1, 1, 0]);
        assert!(world.sps.slots[0].drawn_frame().is_some());
        world.sps_op(0, 130, [-1, 0, 0]);
        assert!(!world.sps.slots[0].loaded && world.sps.slots[0].id == -1);
        world.sps_op(1, 130, [99, 0, 0]);
        assert!(!world.sps.slots[1].loaded, "no such file");
        // A script may name the slot just past the end, which makes one more.
        world.sps_op(16, 130, [43, 0, 0]);
        assert_eq!(world.sps.slots.len(), 17);
        world.sps_op(40, 130, [43, 0, 0]);
        assert_eq!(world.sps.slots.len(), 17);
    }

    #[test]
    fn unload_on_finish_empties_the_slot() {
        let mut world = world();
        world.sps_op(3, 130, [43, 0, 0]);
        world.sps_op(3, 131, [8, 1, 0]);
        world.sps_op(3, 161, [13, 0, 0]);
        world.advance_sps();
        world.advance_sps();
        assert!(!world.sps.slots[3].loaded);
    }

    #[test]
    fn memoria_position_fixes_apply_by_field_and_slot() {
        let mut world = world();
        world.field_map_no = 205;
        world.sps_op(0, 130, [42, 0, 0]);
        // The script gives y negative (up); the effect keeps (x, -y, z) in Unity space.
        world.sps_op(0, 135, [-110, -850, 510]);
        assert_eq!(world.sps.slots[0].pos, [-122.0, 850.0, 510.0]);
        world.sps_op(1, 135, [-110, -850, 510]);
        assert_eq!(
            world.sps.slots[1].pos,
            [-110.0, 850.0, 510.0],
            "only slot 0 is fixed"
        );
        // Treno: an empty slot ignores positions.
        world.field_map_no = 911;
        world.sps_op(2, 135, [5, 5, 5]);
        assert_eq!(world.sps.slots[2].pos, [0.0; 3]);
    }

    #[test]
    fn memoria_enlarges_the_candle_lights_of_fields_50_and_51() {
        let mut world = world();
        world.sps_op(1, 145, [4096, 0, 0]);
        assert_eq!(world.sps.slots[1].scale, 5324, "(int)(4096 * 1.3)");
        world.sps_op(0, 145, [4096, 0, 0]);
        assert_eq!(world.sps.slots[0].scale, 4096, "only slot 1 of field 50");
        world.field_map_no = 51;
        world.sps_op(0, 145, [1000, 0, 0]);
        assert_eq!(world.sps.slots[0].scale, 1300);
        world.field_map_no = 52;
        world.sps_op(0, 145, [1000, 0, 0]);
        assert_eq!(world.sps.slots[0].scale, 1000);
    }
}
