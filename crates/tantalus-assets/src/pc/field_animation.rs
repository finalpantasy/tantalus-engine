//! Per-tick field background animation (milestone W2, runtime side).
//!
//! A pure state machine ported from the pinned Memoria source. It owns no
//! clock: one call of [`FieldAnimation::tick`] is one rendered frame of the
//! field (`FieldMap.HonoUpdate` then `HonoLateUpdate`).
//!
//! * Background animations, `FieldMap.BgAnimationService` (`FieldMap.cs:1538`)
//!   and the `EBG_anim*` setters (`:1011-1105`). `cur_frame` is 8.8 fixed
//!   point, `frame_rate` is the per-step increment (256 is one frame), a frame
//!   holds for `value` ticks (a value of 0 or 1 steps every tick) and a
//!   negative value stops the animation on that frame. An animation steps only
//!   while `Animate` is set and `HasNotFinished` or `Loop` is set, and only on
//!   the field's current camera. Reaching either end clears `HasNotFinished`
//!   and either wraps (default) or reverses (`Palindrome`).
//! * Overlay scrolling, `FieldMap.SceneServiceScroll` (`:1632`) and the
//!   `EBG_overlay*` setters (`:880-1215`): `Loop` (`BGLLOOP`), `Parallax`
//!   (`BGLPARALLAX`), `ScrollWithOffset` (`BGLSCROLLOFFSET`), origin and move
//!   (`BGLORIGIN`, `BGLMOVE`) and timed move.
//! * Overlay placement, `FieldMap.UpdateOverlay` (`:1241`): the wrapping of a
//!   looping overlay's sprites inside its viewport window.
//! * Walkmesh, `WalkMesh.BGI_animShowFrame`, `BGI_floorSetActive`,
//!   `BGI_triSetActive` and the 30 movement simulations `BGI_sim*`
//!   (`WalkMesh.cs:1002-1129`, `:1483-1680`).
//!
//! Assumptions the source does not settle, named instead of hidden:
//!
//! * Camera scroll comes from the caller: placement reads the camera term of
//!   `UpdateOverlay` from [`FieldAnimation::set_screen_offset`] (zero, the
//!   camera at its reference point, until set; `SmoothCamDelta` is taken as
//!   zero). Parallax reads the camera position the caller supplies through
//!   [`FieldAnimation::set_camera_position`].
//! * The 16x9 and per-map patches in `FieldMap` (`ActualPsxScreenWidth` cases,
//!   `fieldmapExtraOffset`, fixes keyed on `fldMapNo`) and in
//!   `EventEngine.DoEventCode` are not applied. Fast forward is off (factor 1).
//! * A fractional position is drawn at the nearest whole field pixel; the
//!   source places quads at subpixel positions.
//! * Overlay depth changes (`MoveTile` z, timed z) are tracked but the layer
//!   order stays the plan's, since the plan sorts once.
//! * Blend modes stay the W1 assumption (`field_composition`).
//! * `BGI_animShowFrame` also relinks neighbor edges and moves characters
//!   (`BGI_animUpdateNeighbors`, `BGI_animUpdateCharacters`); those need the
//!   actor and edge runtime and are not modeled. Simulations report the delta a
//!   rider on the floor would be moved by, but riders are not moved here.
//! * Source quirks are kept as written: `EBG_overlaySetParallax` disables with
//!   `flags &= Parallax`; the Y branch of a negative looping scroll reads
//!   `scrollX`; the non-anchored `ScrollWithOffset` Y anchor reads the window X.
//!   `BGI_animGetFrame` reads `curFrame`, which nothing writes after load.

use crate::install::ImportError;
use crate::pc::bgi::BgiFile;
use crate::pc::bgs::BgsFile;
use crate::pc::field_composition::{AtlasPixels, CameraPlan, Raster, rasterize_camera_placed};
use serde::Serialize;
use std::fmt;

/// `BGANIM_DEF.ANIM_FLAG`.
pub mod anim_flag {
    pub const SINGLE_FRAME: u8 = 1;
    pub const ANIMATE: u8 = 2;
    pub const HAS_NOT_FINISHED: u8 = 4;
    pub const LOOP: u8 = 16;
    pub const PALINDROME: u8 = 32;
    pub const START_PLAY: u8 = ANIMATE | HAS_NOT_FINISHED;
    pub const CONTINUE_PLAY: u8 = HAS_NOT_FINISHED | LOOP;
    pub const MODIFIABLES: u8 = LOOP | PALINDROME;
}

/// `BGOVERLAY_DEF.OVERLAY_FLAG`.
pub mod overlay_flag {
    pub const SCREEN_ANCHORED: u8 = 1;
    pub const ACTIVE: u8 = 2;
    pub const LOOP: u8 = 4;
    pub const PARALLAX: u8 = 8;
    pub const SCROLL_WITH_OFFSET: u8 = 128;
}

/// Number of walkmesh movement simulations (`BGI_simInit`).
pub const SIMULATION_SLOTS: usize = 30;
const VIEWPORTS: usize = 4;
/// `scrollX` value that disables a scroll step (`SceneServiceScroll`).
const SCROLL_DISABLED: i16 = 32767;
const BGI_TRIANGLE_ACTIVE: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldAnimationError {
    /// The source indexes a list with this value and would throw.
    OutOfRange {
        what: &'static str,
        index: i64,
        count: usize,
    },
    Scene(String),
}

impl fmt::Display for FieldAnimationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { what, index, count } => {
                write!(f, "{what} {index} is out of range (count {count})")
            }
            Self::Scene(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for FieldAnimationError {}

fn index(what: &'static str, value: i64, count: usize) -> Result<usize, FieldAnimationError> {
    if value < 0 || value as u64 >= count as u64 {
        return Err(FieldAnimationError::OutOfRange {
            what,
            index: value,
            count,
        });
    }
    Ok(value as usize)
}

/// `BGANIM_DEF` runtime state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationState {
    pub flags: u8,
    pub camera: u8,
    /// 8.8 fixed point; the shown frame is `cur_frame >> 8`.
    pub cur_frame: i32,
    pub frame_rate: i16,
    pub counter: u16,
    /// `(target overlay, ticks to hold)`; a negative hold stops playback.
    pub frames: Vec<(u8, i8)>,
    /// `BGANIM_DEF.actualFrameCount`.
    pub actual_frame_count: i32,
}

impl AnimationState {
    /// The frame index the next service shows.
    pub fn frame(&self) -> i32 {
        self.cur_frame >> 8
    }

    fn calculate_actual_frame_count(&mut self) {
        const DEFAULT_RATE: i32 = 256;
        let count = self.frames.len() as i32;
        let complete = (count - 1) * self.frame_rate as i32;
        let complete_default = (count - 1) * DEFAULT_RATE;
        self.actual_frame_count = count;
        let excess = complete - complete_default;
        if excess.abs() >= DEFAULT_RATE {
            self.actual_frame_count = count - excess / DEFAULT_RATE;
        }
    }

    fn set_frame_rate(&mut self, rate: i32) {
        self.frame_rate = rate as i16;
        self.calculate_actual_frame_count();
    }
}

/// `BGOVERLAY_DEF` runtime state.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayState {
    pub flags: u8,
    pub width: u16,
    pub height: u16,
    pub org_x: f32,
    pub org_y: f32,
    pub cur_x: f32,
    pub cur_y: f32,
    pub org_z: u16,
    pub cur_z: u16,
    pub min_x: i16,
    pub max_x: i16,
    pub min_y: i16,
    pub max_y: i16,
    pub scroll_x: i16,
    pub scroll_y: i16,
    pub is_x_offset: bool,
    pub viewport: u8,
    timed_dx: f32,
    timed_dy: f32,
    timed_dz: i16,
    timed_duration: i32,
}

impl OverlayState {
    pub fn is_active(&self) -> bool {
        self.flags & overlay_flag::ACTIVE != 0
    }
}

/// `scrollWindowPos`, `scrollWindowDim` and `scrollWindowAlpha` of one viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportState {
    pub position: [i16; 2],
    pub size: [i16; 2],
    pub alpha: [i16; 2],
}

/// Background animation, overlay scrolling and viewport state of one field.
#[derive(Debug, Clone)]
pub struct FieldAnimation {
    scene_current: [i32; 2],
    initial_current: Vec<[i32; 2]>,
    pub overlays: Vec<OverlayState>,
    pub animations: Vec<AnimationState>,
    pub viewports: [ViewportState; VIEWPORTS],
    camera: u8,
    camera_position: [f32; 2],
    /// `HalfFieldWidth - realVrp.x`, `HalfFieldHeight - realVrp.y`: where the
    /// background's origin sits on screen (`UpdateOverlay`'s camera term).
    screen_offset: [f32; 2],
    parallax_origin: [f32; 2],
    origin_pending: bool,
    ticks: u64,
}

impl FieldAnimation {
    /// `FieldMap.EBG_init`: every animation starts at its first frame, shown as
    /// a single frame, with that frame's overlay active.
    pub fn new(scene: &BgsFile) -> Result<Self, FieldAnimationError> {
        let overlays: Vec<OverlayState> = scene
            .overlays
            .iter()
            .map(|overlay| OverlayState {
                flags: overlay.flags,
                width: overlay.width,
                height: overlay.height,
                org_x: overlay.origin[0] as f32,
                org_y: overlay.origin[1] as f32,
                cur_x: overlay.current[0] as f32,
                cur_y: overlay.current[1] as f32,
                org_z: overlay.original_z,
                cur_z: overlay.current_z,
                min_x: overlay.limits[0],
                max_x: overlay.limits[1],
                min_y: overlay.limits[2],
                max_y: overlay.limits[3],
                scroll_x: overlay.scroll[0],
                scroll_y: overlay.scroll[1],
                is_x_offset: overlay.is_x_offset,
                viewport: overlay.viewport,
                timed_dx: 0.0,
                timed_dy: 0.0,
                timed_dz: 0,
                timed_duration: 0,
            })
            .collect();
        let mut state = Self {
            scene_current: [scene.current[0] as i32, scene.current[1] as i32],
            initial_current: scene
                .overlays
                .iter()
                .map(|o| [o.current[0] as i32, o.current[1] as i32])
                .collect(),
            overlays,
            animations: Vec::with_capacity(scene.animations.len()),
            viewports: [ViewportState {
                position: [0, 0],
                size: scene
                    .cameras
                    .first()
                    .map_or([0, 0], |camera| [camera.width, camera.height]),
                alpha: [256, 256],
            }; VIEWPORTS],
            camera: 0,
            camera_position: [0.0; 2],
            screen_offset: [0.0; 2],
            parallax_origin: [0.0; 2],
            origin_pending: false,
            ticks: 0,
        };
        for (number, animation) in scene.animations.iter().enumerate() {
            let mut anim = AnimationState {
                flags: anim_flag::SINGLE_FRAME,
                camera: animation.camera,
                cur_frame: 0,
                frame_rate: 256,
                counter: 0,
                frames: animation.frames.clone(),
                actual_frame_count: 0,
            };
            anim.calculate_actual_frame_count();
            let first = anim.frames.first().ok_or_else(|| {
                FieldAnimationError::Scene(format!("animation {number} has no frame 0"))
            })?;
            let target = index("overlay", first.0 as i64, state.overlays.len())?;
            state.overlays[target].flags |= overlay_flag::ACTIVE;
            state.animations.push(anim);
        }
        Ok(state)
    }

    /// Ticks serviced so far.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// `FieldMap.camIdx`: only animations of this camera step.
    pub fn set_camera(&mut self, camera: u8) {
        self.camera = camera;
    }

    pub fn camera(&self) -> u8 {
        self.camera
    }

    /// `FieldMap.curVRP`, read by parallax overlays.
    pub fn set_camera_position(&mut self, position: [f32; 2]) {
        self.camera_position = position;
    }

    /// The camera term of `UpdateOverlay` (`HalfFieldWidth - realVrp.x`,
    /// `HalfFieldHeight - realVrp.y`): the screen position of background pixel
    /// (0, 0), left 0 when the camera sits at its reference point. A screen
    /// anchored looping layer stays put on screen, and every looping layer's
    /// wrap is measured on screen, through it.
    pub fn set_screen_offset(&mut self, offset: [f32; 2]) {
        self.screen_offset = offset;
    }

    /// `FieldMap.parallaxOrg`.
    pub fn set_parallax_origin(&mut self, origin: [f32; 2]) {
        self.parallax_origin = origin;
    }

    /// One field frame: `BgAnimationService`, then `SceneServiceScroll`.
    pub fn tick(&mut self) -> Result<(), FieldAnimationError> {
        self.ticks += 1;
        self.animation_service()?;
        self.scroll_service();
        Ok(())
    }

    /// `FieldMap.BgAnimationService`. A stopping animation ends the whole
    /// service for this tick, as the source `return 1`s from inside the loop.
    pub fn animation_service(&mut self) -> Result<(), FieldAnimationError> {
        for number in 0..self.animations.len() {
            let (running, targets) = {
                let anim = &self.animations[number];
                (
                    anim.flags & anim_flag::ANIMATE != 0
                        && anim.flags & anim_flag::CONTINUE_PLAY != 0
                        && anim.camera == self.camera,
                    anim.frames.iter().map(|f| f.0).collect::<Vec<_>>(),
                )
            };
            if !running {
                continue;
            }
            let overlay_count = self.overlays.len();
            for target in &targets {
                let target = index("overlay", *target as i64, overlay_count)?;
                self.overlays[target].flags &= !overlay_flag::ACTIVE;
            }
            let anim = &mut self.animations[number];
            let current = index(
                "animation frame",
                (anim.cur_frame >> 8) as i64,
                targets.len(),
            )?;
            let target = index("overlay", targets[current] as i64, overlay_count)?;
            self.overlays[target].flags |= overlay_flag::ACTIVE;
            if anim.frames[current].1 < 0 {
                anim.frames[current].1 = 0;
                anim.counter = 0;
                anim.flags &= !anim_flag::HAS_NOT_FINISHED;
                return Ok(());
            }
            anim.counter = anim.counter.wrapping_add(1);
            if anim.counter as i32 >= anim.frames[current].1 as i32 {
                anim.counter = 0;
                anim.cur_frame = anim.cur_frame.wrapping_add(anim.frame_rate as i32);
                let shown = anim.cur_frame >> 8;
                let count = anim.frames.len() as i32;
                if shown >= count {
                    if anim.flags & anim_flag::PALINDROME != 0 {
                        anim.cur_frame = (count - 1) << 8;
                        anim.set_frame_rate(-(anim.frame_rate as i32));
                    } else {
                        anim.cur_frame = 0;
                    }
                    anim.flags &= !anim_flag::HAS_NOT_FINISHED;
                } else if shown < 0 {
                    if anim.flags & anim_flag::PALINDROME != 0 {
                        anim.cur_frame = 0;
                        anim.set_frame_rate(-(anim.frame_rate as i32));
                    } else {
                        anim.cur_frame = (count - 1) << 8;
                    }
                    anim.flags &= !anim_flag::HAS_NOT_FINISHED;
                }
            }
        }
        Ok(())
    }

    /// `FieldMap.SceneServiceScroll` for every overlay.
    pub fn scroll_service(&mut self) {
        let camera = self.camera_position;
        let origin = self.parallax_origin;
        for overlay in &mut self.overlays {
            if overlay.flags & overlay_flag::LOOP != 0 {
                if overlay.scroll_x != 0 && overlay.scroll_x != SCROLL_DISABLED {
                    overlay.cur_x =
                        modulo(overlay.cur_x, overlay.width) + overlay.scroll_x as f32 / 256.0;
                }
                if overlay.scroll_y != 0 && overlay.scroll_y != SCROLL_DISABLED {
                    overlay.cur_y =
                        modulo(overlay.cur_y, overlay.height) + overlay.scroll_y as f32 / 256.0;
                }
            }
            if overlay.flags & overlay_flag::SCROLL_WITH_OFFSET != 0 {
                if overlay.is_x_offset {
                    if overlay.scroll_y != SCROLL_DISABLED {
                        overlay.cur_y =
                            modulo(overlay.cur_y, overlay.height) + overlay.scroll_y as f32 / 256.0;
                    }
                } else if overlay.scroll_x != SCROLL_DISABLED {
                    overlay.cur_x =
                        modulo(overlay.cur_x, overlay.width) + overlay.scroll_x as f32 / 256.0;
                }
            }
            if overlay.flags & overlay_flag::PARALLAX != 0 {
                overlay.cur_x =
                    overlay.org_x + (camera[0] - origin[0]) * (overlay.scroll_x as f32 / 256.0);
                overlay.cur_y =
                    overlay.org_y + (camera[1] - origin[1]) * (overlay.scroll_y as f32 / 256.0);
            }
            if overlay.timed_duration > 0 {
                overlay.org_x += overlay.timed_dx;
                overlay.org_y += overlay.timed_dy;
                overlay.org_z = (overlay.org_z as i32 + overlay.timed_dz as i32)
                    .clamp(0, u16::MAX as i32) as u16;
                overlay.cur_x = overlay.org_x;
                overlay.cur_y = overlay.org_y;
                overlay.cur_z = overlay.org_z;
                overlay.timed_duration -= 1;
            }
        }
        if self.origin_pending {
            self.parallax_origin = self.camera_position;
            self.origin_pending = false;
        }
    }

    fn overlay_mut(&mut self, overlay: i64) -> Result<&mut OverlayState, FieldAnimationError> {
        let at = index("overlay", overlay, self.overlays.len())?;
        Ok(&mut self.overlays[at])
    }

    fn animation_mut(
        &mut self,
        animation: i64,
    ) -> Result<&mut AnimationState, FieldAnimationError> {
        let at = index("animation", animation, self.animations.len())?;
        Ok(&mut self.animations[at])
    }

    /// `EBG_overlaySetActive` (`BGLACTIVE`).
    pub fn overlay_set_active(
        &mut self,
        overlay: i64,
        active: bool,
    ) -> Result<(), FieldAnimationError> {
        let overlay = self.overlay_mut(overlay)?;
        if active {
            overlay.flags |= overlay_flag::ACTIVE;
        } else {
            overlay.flags &= !overlay_flag::ACTIVE;
        }
        Ok(())
    }

    /// `EBG_overlayDefineViewport` (`BGVDEFINE`); an index outside 0..4 is 0.
    pub fn overlay_define_viewport(&mut self, viewport: i64, position: [i16; 2], size: [i16; 2]) {
        let viewport = if (0..VIEWPORTS as i64).contains(&viewport) {
            viewport as usize
        } else {
            0
        };
        self.viewports[viewport].position = position;
        self.viewports[viewport].size = size;
    }

    /// `EBG_overlayDefineViewportAlpha` (`BGVALPHA`).
    pub fn overlay_define_viewport_alpha(
        &mut self,
        viewport: i64,
        alpha: [i16; 2],
    ) -> Result<(), FieldAnimationError> {
        let viewport = index("viewport", viewport, VIEWPORTS)?;
        self.viewports[viewport].alpha = alpha;
        Ok(())
    }

    /// `EBG_overlaySetViewport` (`BGVSET`).
    pub fn overlay_set_viewport(
        &mut self,
        overlay: i64,
        viewport: i64,
    ) -> Result<(), FieldAnimationError> {
        self.overlay_mut(overlay)?.viewport = viewport as u8;
        Ok(())
    }

    /// `EBG_overlaySetLoop` (`BGLLOOP`).
    pub fn overlay_set_loop(
        &mut self,
        overlay: i64,
        enabled: bool,
        speed: [i16; 2],
    ) -> Result<(), FieldAnimationError> {
        let overlay = self.overlay_mut(overlay)?;
        if enabled {
            overlay.flags |= overlay_flag::LOOP;
        } else {
            overlay.flags &= !overlay_flag::LOOP;
        }
        overlay.scroll_x = speed[0];
        overlay.scroll_y = speed[1];
        Ok(())
    }

    /// `EBG_overlaySetLoopType` (`BGLLOOPTYPE`).
    pub fn overlay_set_loop_type(
        &mut self,
        overlay: i64,
        screen_anchored: bool,
    ) -> Result<(), FieldAnimationError> {
        let overlay = self.overlay_mut(overlay)?;
        if screen_anchored {
            overlay.flags |= overlay_flag::SCREEN_ANCHORED;
        } else {
            overlay.flags &= !overlay_flag::SCREEN_ANCHORED;
        }
        Ok(())
    }

    /// `EBG_overlaySetScrollWithOffset` (`BGLSCROLLOFFSET`).
    pub fn overlay_set_scroll_with_offset(
        &mut self,
        overlay: i64,
        enabled: bool,
        delta: i16,
        offset: i16,
        is_x_offset: bool,
    ) -> Result<(), FieldAnimationError> {
        let overlay = self.overlay_mut(overlay)?;
        if enabled {
            overlay.flags |= overlay_flag::SCROLL_WITH_OFFSET;
        } else {
            overlay.flags &= !overlay_flag::SCROLL_WITH_OFFSET;
        }
        if is_x_offset {
            overlay.scroll_x = offset;
            overlay.scroll_y = delta;
        } else {
            overlay.scroll_x = delta;
            overlay.scroll_y = offset;
        }
        overlay.is_x_offset = is_x_offset;
        Ok(())
    }

    /// `EBG_overlaySetParallax` (`BGLPARALLAX`). Disabling keeps only the
    /// Parallax bit (`flags &= Parallax`), exactly as the source does.
    pub fn overlay_set_parallax(
        &mut self,
        overlay: i64,
        enabled: bool,
        speed: [i16; 2],
    ) -> Result<(), FieldAnimationError> {
        let overlay = self.overlay_mut(overlay)?;
        if enabled {
            overlay.flags |= overlay_flag::PARALLAX;
        } else {
            overlay.flags &= overlay_flag::PARALLAX;
        }
        overlay.scroll_x = speed[0];
        overlay.scroll_y = speed[1];
        Ok(())
    }

    /// `EBG_overlaySetOrigin` (`BGLORIGIN`).
    pub fn overlay_set_origin(
        &mut self,
        overlay: i64,
        x: f32,
        y: f32,
    ) -> Result<(), FieldAnimationError> {
        let overlay = self.overlay_mut(overlay)?;
        overlay.cur_x = x;
        overlay.cur_y = y;
        overlay.org_x = x;
        overlay.org_y = y;
        self.origin_pending = true;
        Ok(())
    }

    /// `EBG_overlayMove` (`BGLMOVE`): origin plus delta, clamped to the limits.
    pub fn overlay_move(
        &mut self,
        overlay: i64,
        dx: f32,
        dy: f32,
        dz: i16,
    ) -> Result<(), FieldAnimationError> {
        let overlay = self.overlay_mut(overlay)?;
        let dest_x = unity_clamp(
            overlay.org_x + dx,
            overlay.min_x as f32,
            overlay.max_x as f32,
        );
        let dest_y = unity_clamp(
            overlay.org_y + dy,
            overlay.min_y as f32,
            overlay.max_y as f32,
        );
        let dest_z = overlay.org_z.wrapping_add(dz as u16);
        overlay.org_x = dest_x;
        overlay.org_y = dest_y;
        overlay.org_z = dest_z;
        overlay.cur_x = dest_x;
        overlay.cur_y = dest_y;
        overlay.cur_z = dest_z;
        Ok(())
    }

    /// `EBG_overlayMoveTimed`: spread a move over `ticks` ticks.
    pub fn overlay_move_timed(
        &mut self,
        overlay: i64,
        dx: f32,
        dy: f32,
        dz: i16,
        ticks: i32,
    ) -> Result<(), FieldAnimationError> {
        let overlay = self.overlay_mut(overlay)?;
        if ticks == 0 {
            return Err(FieldAnimationError::Scene(
                "timed move over 0 ticks divides by zero".to_owned(),
            ));
        }
        overlay.timed_dx = dx / ticks as f32;
        overlay.timed_dy = dy / ticks as f32;
        overlay.timed_dz = (dz as i32 / ticks) as i16;
        overlay.timed_duration = ticks;
        Ok(())
    }

    /// `EBG_animAnimate` (`BGAANIME`): sets `Animate` and starts at a frame.
    /// The animation steps only once `HasNotFinished` or `Loop` is also set.
    pub fn anim_animate(&mut self, animation: i64, frame: i32) -> Result<(), FieldAnimationError> {
        let anim = self.animation_mut(animation)?;
        anim.flags |= anim_flag::ANIMATE;
        anim.cur_frame = frame << 8;
        anim.counter = 0;
        Ok(())
    }

    /// `EBG_animShowFrame` (`BGAFRAME`): only the frame's overlay is active; a
    /// frame number past the list hides them all.
    pub fn anim_show_frame(
        &mut self,
        animation: i64,
        frame: i64,
    ) -> Result<(), FieldAnimationError> {
        let at = index("animation", animation, self.animations.len())?;
        let targets: Vec<u8> = self.animations[at].frames.iter().map(|f| f.0).collect();
        for (number, target) in targets.iter().enumerate() {
            let overlay = index("overlay", *target as i64, self.overlays.len())?;
            if number as i64 == frame {
                self.overlays[overlay].flags |= overlay_flag::ACTIVE;
            } else {
                self.overlays[overlay].flags &= !overlay_flag::ACTIVE;
            }
        }
        self.animations[at].flags = anim_flag::SINGLE_FRAME;
        Ok(())
    }

    /// `EBG_animSetActive` (`BGAACTIVE`).
    pub fn anim_set_active(
        &mut self,
        animation: i64,
        active: bool,
    ) -> Result<(), FieldAnimationError> {
        let anim = self.animation_mut(animation)?;
        if active {
            anim.flags |= anim_flag::START_PLAY;
        } else {
            anim.flags &= !anim_flag::START_PLAY;
        }
        Ok(())
    }

    /// `EBG_animSetFrameRate` (`BGARATE`).
    pub fn anim_set_frame_rate(
        &mut self,
        animation: i64,
        rate: i32,
    ) -> Result<(), FieldAnimationError> {
        self.animation_mut(animation)?.set_frame_rate(rate);
        Ok(())
    }

    /// `EBG_animSetFrameWait` (`BGAWAIT`).
    pub fn anim_set_frame_wait(
        &mut self,
        animation: i64,
        frame: i64,
        wait: i32,
    ) -> Result<(), FieldAnimationError> {
        let anim = self.animation_mut(animation)?;
        let frame = index("animation frame", frame, anim.frames.len())?;
        anim.frames[frame].1 = wait as i8;
        Ok(())
    }

    /// `EBG_animSetFlags` (`BGAFLAG`): adds only `Loop` and `Palindrome`.
    pub fn anim_set_flags(
        &mut self,
        animation: i64,
        flags: i32,
    ) -> Result<(), FieldAnimationError> {
        self.animation_mut(animation)?.flags |= (flags as u8) & anim_flag::MODIFIABLES;
        Ok(())
    }

    /// `EBG_animSetPlayRange` (`BGARANGE`): the last frame gets a stop marker.
    pub fn anim_set_play_range(
        &mut self,
        animation: i64,
        first: i32,
        last: i64,
    ) -> Result<(), FieldAnimationError> {
        let anim = self.animation_mut(animation)?;
        anim.flags |= anim_flag::START_PLAY;
        anim.cur_frame = first << 8;
        anim.counter = 0;
        let last_at = index("animation frame", last, anim.frames.len())?;
        anim.frames[last_at].1 = -1;
        if (anim.frame_rate > 0 && last < first as i64)
            || (anim.frame_rate < 0 && last > first as i64)
        {
            anim.frame_rate = (-(anim.frame_rate as i32)) as i16;
        }
        Ok(())
    }

    /// `EBG_animSetVisible` (`BGAVISIBLE`): reset, optionally showing frame 0.
    pub fn anim_set_visible(
        &mut self,
        animation: i64,
        visible: bool,
    ) -> Result<(), FieldAnimationError> {
        let at = index("animation", animation, self.animations.len())?;
        {
            let anim = &mut self.animations[at];
            anim.cur_frame = 0;
            anim.frame_rate = 256;
            anim.counter = 0;
            anim.flags = anim_flag::SINGLE_FRAME;
        }
        let targets: Vec<u8> = self.animations[at].frames.iter().map(|f| f.0).collect();
        for (number, target) in targets.iter().enumerate() {
            let overlay = index("overlay", *target as i64, self.overlays.len())?;
            if visible && number == 0 {
                self.overlays[overlay].flags |= overlay_flag::ACTIVE;
            } else {
                self.overlays[overlay].flags &= !overlay_flag::ACTIVE;
            }
        }
        Ok(())
    }

    /// Frame index whose overlay an animation shows right now: the last frame
    /// in list order whose target overlay is active, if any.
    pub fn visible_frame(&self, animation: usize) -> Option<usize> {
        let anim = self.animations.get(animation)?;
        anim.frames.iter().rposition(|(target, _)| {
            self.overlays
                .get(*target as usize)
                .is_some_and(|o| o.is_active())
        })
    }

    /// Whether an overlay is drawn now.
    pub fn overlay_drawn(&self, overlay: usize) -> bool {
        self.overlays
            .get(overlay)
            .is_some_and(OverlayState::is_active)
    }

    /// Field position of one tile, in whole field pixels. `plan_position` is
    /// the tile's initial `[x, y]` from the layer plan (scene origin + overlay
    /// origin + sprite offset). Port of `UpdateOverlay` under the camera
    /// assumptions in the module notes.
    pub fn tile_position(
        &self,
        overlay: usize,
        plan_position: [i32; 2],
    ) -> Result<[i32; 2], FieldAnimationError> {
        let at = index("overlay", overlay as i64, self.overlays.len())?;
        let o = &self.overlays[at];
        let initial = self.initial_current[at];
        let off_x = (plan_position[0] - self.scene_current[0] - initial[0]) as f32;
        let off_y = (plan_position[1] - self.scene_current[1] - initial[1]) as f32;
        let scene_x = self.scene_current[0] as f32;
        let scene_y = self.scene_current[1] as f32;
        let (w, h) = (o.width as f32, o.height as f32);
        // Only a looping or offset-scrolling overlay reads its viewport; the
        // source indexes the four windows there and would throw past them.
        let window = || -> Result<ViewportState, FieldAnimationError> {
            Ok(self.viewports[index("viewport", o.viewport as i64, VIEWPORTS)?])
        };
        // `scene.scrX` is `scene.curX + HalfFieldWidth - realVrp.x`; the
        // sprite's local position subtracts it again (`cacheLocalPos.x -=
        // scrX + curX`), so the camera term `[cx, cy]` stays only where an
        // anchor or a wrap does not take it back out.
        let [cx, cy] = self.screen_offset;
        let mut screen_x = o.cur_x + scene_x + cx;
        let mut screen_y = o.cur_y + scene_y + cy;
        let anchored = o.flags & overlay_flag::SCREEN_ANCHORED != 0;
        let position = if o.flags & overlay_flag::LOOP != 0 {
            let window = window()?;
            let (window_x, window_y) = (window.position[0] as f32, window.position[1] as f32);
            let (dim_w, dim_h) = (window.size[0] as f32, window.size[1] as f32);
            if o.scroll_x != 0 {
                if o.scroll_x < 0 {
                    let delta = (256 - o.scroll_x as i32) as i16 as i32;
                    screen_x = ((((o.cur_x as i32) << 8) + delta) >> 8) as f32 + scene_x + cx;
                }
                screen_x = wrap_in_window(screen_x, dim_w, w);
            }
            if o.scroll_y != 0 {
                if o.scroll_y < 0 {
                    // The source reads scrollX here.
                    let delta = (256 - o.scroll_x as i32) as i16 as i32;
                    screen_y = ((((o.cur_y as i32) << 8) + delta) >> 8) as f32 + scene_y + cy;
                }
                screen_y = wrap_in_window(screen_y, dim_h, h);
            }
            let mut x = screen_x + off_x;
            let mut y = screen_y + off_y;
            if o.scroll_x != 0 {
                x = wrap_sprite(x, w);
            }
            if o.scroll_y != 0 {
                y = wrap_sprite(y, h);
            }
            // The anchor: the window alone when screen anchored, the window
            // plus the camera term otherwise, and nothing on a still axis of
            // a layer that is not anchored.
            let place = |at: f32, scrolls: bool, window: f32, camera: f32| {
                if anchored {
                    at + window - camera
                } else if scrolls {
                    at + window
                } else {
                    at - camera
                }
            };
            [
                place(x, o.scroll_x != 0, window_x, cx),
                place(y, o.scroll_y != 0, window_y, cy),
            ]
        } else if o.flags & overlay_flag::SCROLL_WITH_OFFSET != 0 {
            let window = window()?;
            let (window_x, window_y) = (window.position[0] as f32, window.position[1] as f32);
            let (dim_w, dim_h) = (window.size[0] as f32, window.size[1] as f32);
            // The non-anchored branch adds the camera term, and reads the
            // window X for the Y anchor.
            let anchor_x = if anchored { window_x } else { window_x + cx };
            let anchor_y = if anchored { window_y } else { window_x + cy };
            if o.is_x_offset {
                screen_y = wrap_in_window(screen_y, dim_h, h);
                screen_x += float_mod(screen_y * o.scroll_x as f32 / h, w);
                let mut y_offset = screen_y + off_y;
                let mut x_offset = 0.0;
                if y_offset + 16.0 >= h {
                    y_offset -= h;
                    x_offset = -(o.scroll_x as f32);
                } else if y_offset <= -16.0 {
                    y_offset += h;
                    x_offset = o.scroll_x as f32;
                }
                [screen_x + off_x + x_offset - cx, y_offset + anchor_y - cy]
            } else {
                screen_x = wrap_in_window(screen_x, dim_w, w);
                screen_y += float_mod(screen_x * o.scroll_y as f32 / w, h);
                let mut x_adjusted = screen_x + off_x;
                let mut y_offset = 0.0;
                if x_adjusted + 16.0 >= w {
                    x_adjusted -= w;
                    y_offset = -(o.scroll_y as f32);
                } else if x_adjusted <= -16.0 {
                    x_adjusted += w;
                    y_offset = o.scroll_y as f32;
                }
                [x_adjusted + anchor_x - cx, screen_y + off_y + y_offset - cy]
            }
        } else {
            [screen_x + off_x - cx, screen_y + off_y - cy]
        };
        Ok([
            (position[0] + 0.5).floor() as i32,
            (position[1] + 0.5).floor() as i32,
        ])
    }
}

/// C# `float % float` with a zero divisor left alone instead of NaN.
fn float_mod(value: f32, divisor: f32) -> f32 {
    if divisor == 0.0 {
        value
    } else {
        value % divisor
    }
}

fn modulo(value: f32, size: u16) -> f32 {
    float_mod(value, size as f32)
}

/// `(screen - (dim - size)) % size + (dim - size)`.
fn wrap_in_window(screen: f32, dim: f32, size: f32) -> f32 {
    float_mod(screen - (dim - size), size) + (dim - size)
}

/// The per-sprite wrap of a looping axis.
fn wrap_sprite(position: f32, size: f32) -> f32 {
    if position + 16.0 >= size {
        position - size
    } else if position <= -16.0 {
        position + size
    } else {
        position
    }
}

/// `Mathf.Clamp`: a value under the minimum wins first.
fn unity_clamp(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// Rasterizes one camera of a field at the animation's current state:
/// overlays drawn are the active ones, each tile at `tile_position`.
pub fn rasterize_state(
    camera: &CameraPlan,
    atlas: &AtlasPixels<'_>,
    state: &FieldAnimation,
) -> Result<Option<Raster>, ImportError> {
    rasterize_camera_placed(
        camera,
        atlas,
        |layer| state.overlay_drawn(layer.overlay_index as usize),
        |layer, tile| {
            state
                .tile_position(layer.overlay_index as usize, [tile.x, tile.y])
                .map_err(|error| ImportError::invalid("field-animation", error.to_string()))
        },
    )
}

/// `BGI_FLOOR_DEF` runtime position of one floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FloorPosition {
    pub original: [i16; 3],
    pub current: [i16; 3],
}

/// `BGI_SIM_DEF`: one walkmesh floor movement simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Simulation {
    pub flags: u16,
    pub floor: u16,
    pub axis: u16,
    pub algorithm: u16,
    pub delta_min: i16,
    pub delta_max: i16,
    pub delta_current: i16,
    pub delta_previous: i16,
    pub frame_count: u16,
    pub frame_rate: i16,
    pub frame_index: i32,
}

impl Simulation {
    fn new() -> Self {
        Self {
            flags: 2,
            floor: 0,
            axis: 0,
            algorithm: 0,
            delta_min: 0,
            delta_max: 0,
            delta_current: 0,
            delta_previous: 0,
            frame_count: 0,
            frame_rate: 256,
            frame_index: 0,
        }
    }

    /// `BGI_simEvaluate`.
    fn evaluate(&mut self) {
        self.frame_index = self.frame_index.wrapping_add(self.frame_rate as i32);
        let step = self.frame_index >> 8;
        let span = self.delta_max as i32 - self.delta_min as i32;
        if !(self.frame_count > 0 && self.frame_rate != 0 && span > 0) {
            return;
        }
        let value = if self.algorithm == 1 {
            let angle = ((((step << 12) / span) as i16) as i32) + 2048;
            let cosine = (bgi_cos(angle) + 4096.0) as i32;
            (self.delta_min as f32 + (span * cosine) as f32 / 8192.0) as i32
        } else {
            self.delta_min as i32 + step
        };
        self.delta_previous = self.delta_current;
        self.delta_current = value as i16;
        if self.delta_current > self.delta_max {
            if self.flags & 16 != 0 {
                self.delta_current = self.delta_max;
                self.frame_rate = (-(self.frame_rate as i32)) as i16;
            } else {
                self.delta_current = self.delta_min;
            }
            self.frame_index = (span - 1) << 8;
            self.flags &= 0xfffb;
        } else if self.delta_current < self.delta_min {
            if self.flags & 16 != 0 {
                self.delta_current = self.delta_min;
                self.frame_rate = (-(self.frame_rate as i32)) as i16;
            } else {
                self.delta_current = self.delta_max;
            }
            self.frame_index = 0;
            self.flags &= 0xfffb;
        }
    }
}

/// `BGI.BGI_rcos`: 4096 units per turn, result scaled by 4096.
fn bgi_cos(angle: i32) -> f32 {
    let radians = angle as f32 / 4096.0 * 360.0 * 0.017_453_292;
    ((radians as f64).cos() as f32) * 4096.0
}

/// What `BGI_animShowFrame` changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShownFrame {
    /// Frame that carried an active triangle before the call, if any.
    pub previous_frame: Option<u16>,
    /// The shown frame differs, so the source relinks neighbors and moves
    /// characters (not modeled here).
    pub changed: bool,
}

/// Walkmesh animation, floor and triangle toggles and simulations.
#[derive(Debug, Clone)]
pub struct WalkmeshAnimation {
    triangle_flags: Vec<u16>,
    floor_flags: Vec<u16>,
    floor_triangles: Vec<Vec<i32>>,
    floors: Vec<FloorPosition>,
    animation_frames: Vec<Vec<Vec<i32>>>,
    animation_start_frames: Vec<i32>,
    simulations: [Simulation; SIMULATION_SLOTS],
}

impl WalkmeshAnimation {
    /// Loaded state of a `.bgi`; simulations start as `BGI_simInit` sets them.
    pub fn new(bgi: &BgiFile) -> Self {
        Self {
            triangle_flags: bgi.triangles.iter().map(|t| t.flags).collect(),
            floor_flags: bgi.floors.iter().map(|f| f.flags).collect(),
            floor_triangles: bgi.floors.iter().map(|f| f.triangles.clone()).collect(),
            floors: bgi
                .floors
                .iter()
                .map(|f| FloorPosition {
                    original: f.bounds[0],
                    current: f.bounds[1],
                })
                .collect(),
            animation_frames: bgi.animations.iter().map(|a| a.triangles.clone()).collect(),
            animation_start_frames: bgi.animations.iter().map(|a| a.current_frame).collect(),
            simulations: [Simulation::new(); SIMULATION_SLOTS],
        }
    }

    pub fn triangle_active(&self, triangle: usize) -> bool {
        self.triangle_flags
            .get(triangle)
            .is_some_and(|flags| flags & BGI_TRIANGLE_ACTIVE != 0)
    }

    pub fn floor_active(&self, floor: usize) -> bool {
        self.floor_flags
            .get(floor)
            .is_some_and(|flags| flags & BGI_TRIANGLE_ACTIVE != 0)
    }

    pub fn floor_position(&self, floor: usize) -> Option<FloorPosition> {
        self.floors.get(floor).copied()
    }

    pub fn simulation(&self, simulation: usize) -> Option<&Simulation> {
        self.simulations.get(simulation)
    }

    /// `BGI_floorSetActive`; `false` when the floor does not exist.
    pub fn floor_set_active(&mut self, floor: u32, active: bool) -> bool {
        let floor = floor as usize;
        if floor >= self.floor_flags.len() {
            return false;
        }
        for &triangle in &self.floor_triangles[floor] {
            let flags = &mut self.triangle_flags[triangle as usize];
            if active {
                *flags |= BGI_TRIANGLE_ACTIVE;
            } else {
                *flags &= !BGI_TRIANGLE_ACTIVE;
            }
        }
        if active {
            self.floor_flags[floor] |= BGI_TRIANGLE_ACTIVE;
        } else {
            self.floor_flags[floor] &= !BGI_TRIANGLE_ACTIVE;
        }
        true
    }

    /// `BGI_triSetActive`.
    pub fn triangle_set_active(
        &mut self,
        triangle: u32,
        active: bool,
    ) -> Result<(), FieldAnimationError> {
        let at = index(
            "walkmesh triangle",
            triangle as i64,
            self.triangle_flags.len(),
        )?;
        if active {
            self.triangle_flags[at] |= BGI_TRIANGLE_ACTIVE;
        } else {
            self.triangle_flags[at] &= !BGI_TRIANGLE_ACTIVE;
        }
        Ok(())
    }

    /// `BGI_animShowFrame`; `None` when the animation or frame does not exist.
    pub fn animation_show_frame(&mut self, animation: u32, frame: u32) -> Option<ShownFrame> {
        let frames = self.animation_frames.get(animation as usize)?;
        if frame as usize >= frames.len() {
            return None;
        }
        let mut previous = None;
        for (number, triangles) in frames.iter().enumerate() {
            for &triangle in triangles {
                let flags = &mut self.triangle_flags[triangle as usize];
                if *flags & BGI_TRIANGLE_ACTIVE != 0 {
                    previous = Some(number as u16);
                }
                *flags &= !BGI_TRIANGLE_ACTIVE;
            }
        }
        for &triangle in &frames[frame as usize] {
            self.triangle_flags[triangle as usize] |= BGI_TRIANGLE_ACTIVE;
        }
        Some(ShownFrame {
            previous_frame: previous,
            changed: previous != Some(frame as u16),
        })
    }

    /// `BGI_animGetFrame`: `curFrame >> 8` as loaded; nothing else writes it.
    pub fn animation_frame(&self, animation: u32) -> Option<i32> {
        self.animation_start_frames
            .get(animation as usize)
            .map(|frame| frame >> 8)
    }

    fn simulation_mut(&mut self, simulation: u32) -> Result<&mut Simulation, FieldAnimationError> {
        let at = index("simulation", simulation as i64, SIMULATION_SLOTS)?;
        Ok(&mut self.simulations[at])
    }

    /// `BGI_simSetFloor`.
    pub fn simulation_set_floor(
        &mut self,
        simulation: u32,
        floor: u32,
    ) -> Result<(), FieldAnimationError> {
        self.simulation_mut(simulation)?.floor = floor as u16;
        Ok(())
    }

    /// `BGI_simSetActive`: starts or stops, then evaluates once either way.
    pub fn simulation_set_active(
        &mut self,
        simulation: u32,
        active: bool,
    ) -> Result<(), FieldAnimationError> {
        let sim = self.simulation_mut(simulation)?;
        if active {
            sim.flags |= 5;
        } else {
            sim.flags &= 0xfffa;
        }
        sim.evaluate();
        Ok(())
    }

    /// `BGI_simSetFlags`: replaces bits 8 and 16 (0x18) only.
    pub fn simulation_set_flags(
        &mut self,
        simulation: u32,
        flags: u32,
    ) -> Result<(), FieldAnimationError> {
        let sim = self.simulation_mut(simulation)?;
        sim.flags &= 0xffe7;
        sim.flags |= (flags & 24) as u16;
        Ok(())
    }

    /// `BGI_simSetFrameRate`.
    pub fn simulation_set_frame_rate(
        &mut self,
        simulation: u32,
        rate: i16,
    ) -> Result<(), FieldAnimationError> {
        self.simulation_mut(simulation)?.frame_rate = rate;
        Ok(())
    }

    /// `BGI_simSetAlgorithm`.
    pub fn simulation_set_algorithm(
        &mut self,
        simulation: u32,
        algorithm: u32,
    ) -> Result<(), FieldAnimationError> {
        self.simulation_mut(simulation)?.algorithm = algorithm as u16;
        Ok(())
    }

    /// `BGI_simSetDelta`.
    pub fn simulation_set_delta(
        &mut self,
        simulation: u32,
        min: i16,
        max: i16,
    ) -> Result<(), FieldAnimationError> {
        let sim = self.simulation_mut(simulation)?;
        sim.delta_min = min;
        sim.delta_max = max;
        sim.frame_count = (max as i32 - min as i32) as u16;
        Ok(())
    }

    /// `BGI_simSetAxis`.
    pub fn simulation_set_axis(
        &mut self,
        simulation: u32,
        axis: u32,
    ) -> Result<(), FieldAnimationError> {
        self.simulation_mut(simulation)?.axis = axis as u16;
        Ok(())
    }

    /// `BGI_simService`: one walkmesh tick for all 30 simulations.
    pub fn tick(&mut self) -> Result<(), FieldAnimationError> {
        for number in 0..SIMULATION_SLOTS {
            let sim = self.simulations[number];
            if sim.flags & 1 == 0 || sim.flags & 12 == 0 {
                continue;
            }
            let floor = index("walkmesh floor", sim.floor as i64, self.floors.len())?;
            let axis = index("walkmesh axis", sim.axis as i64, 3)?;
            let position = &mut self.floors[floor];
            position.current[axis] = position.original[axis].wrapping_sub(sim.delta_current);
            self.simulations[number].evaluate();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pc::bgs::{BgsAnimation, BgsCamera, BgsOverlay};

    fn overlay(camera: u8, flags: u8) -> BgsOverlay {
        BgsOverlay {
            flags,
            current_z: 0,
            original_z: 0,
            width: 64,
            height: 32,
            origin: [0; 2],
            current: [0; 2],
            limits: [-1000, 1000, -1000, 1000],
            screen: [0; 2],
            scroll: [0; 2],
            fraction: [0; 2],
            camera,
            is_x_offset: false,
            viewport: 0,
            sprites: Vec::new(),
        }
    }

    fn scene(overlays: Vec<BgsOverlay>, animations: Vec<Vec<(u8, i8)>>) -> BgsFile {
        BgsFile {
            scene_length: 0,
            depth_bit_shift: 1,
            origin_z: 0,
            current_z: 0,
            origin: [0; 2],
            current: [0; 2],
            limits: [0; 4],
            screen: [0; 2],
            animations: animations
                .into_iter()
                .map(|frames| BgsAnimation {
                    flags: 1,
                    camera: 0,
                    current_frame: 0,
                    frame_rate: 256,
                    counter: 0,
                    frames,
                })
                .collect(),
            overlays,
            lights: Vec::new(),
            cameras: vec![BgsCamera {
                projection: 100,
                rotation: [[0; 3]; 3],
                translation: [0; 3],
                center_offset: [0; 2],
                width: 320,
                height: 224,
                viewport_x: [0; 2],
                viewport_y: [0; 2],
                depth_offset: 0,
            }],
        }
    }

    /// Overlays 0..n, one per frame, plus an animation over them.
    fn animated(waits: &[i8]) -> FieldAnimation {
        let overlays = (0..waits.len()).map(|_| overlay(0, 0)).collect();
        let frames = waits
            .iter()
            .enumerate()
            .map(|(number, wait)| (number as u8, *wait))
            .collect();
        FieldAnimation::new(&scene(overlays, vec![frames])).unwrap()
    }

    fn frames_after(state: &mut FieldAnimation, ticks: usize) -> Vec<i32> {
        (0..ticks)
            .map(|_| {
                state.tick().unwrap();
                state.animations[0].frame()
            })
            .collect()
    }

    #[test]
    fn init_shows_only_the_first_frame_and_does_not_run() {
        let mut state = animated(&[2, 1, 3]);
        assert_eq!(state.animations[0].flags, anim_flag::SINGLE_FRAME);
        assert_eq!(state.visible_frame(0), Some(0));
        assert!(!state.overlay_drawn(1));
        assert_eq!(frames_after(&mut state, 5), [0; 5]);
    }

    #[test]
    fn one_shot_holds_each_frame_for_its_ticks_and_then_stops() {
        // Holds 2, 1, 3. Hand-derived: tick 2 steps to frame 1, tick 3 to 2,
        // tick 6 past the end: frame 0 again with HasNotFinished cleared.
        let mut state = animated(&[2, 1, 3]);
        state.anim_set_active(0, true).unwrap();
        assert_eq!(state.animations[0].flags, 1 | 2 | 4);
        assert_eq!(frames_after(&mut state, 8), [0, 1, 2, 2, 2, 0, 0, 0]);
        assert_eq!(state.animations[0].flags & anim_flag::HAS_NOT_FINISHED, 0);
        // The overlay shown is the one chosen at the start of the last service.
        assert_eq!(state.visible_frame(0), Some(2));
    }

    #[test]
    fn shown_overlay_lags_the_frame_index_by_one_service() {
        let mut state = animated(&[1, 1, 1]);
        state.anim_set_active(0, true).unwrap();
        state.tick().unwrap();
        assert_eq!(state.animations[0].frame(), 1);
        assert_eq!(state.visible_frame(0), Some(0));
        state.tick().unwrap();
        assert_eq!(state.visible_frame(0), Some(1));
    }

    #[test]
    fn animate_alone_does_not_run_until_loop_or_start() {
        let mut state = animated(&[1, 1, 1]);
        state.anim_animate(0, 1).unwrap();
        assert_eq!(state.animations[0].cur_frame, 256);
        assert_eq!(frames_after(&mut state, 2), [1, 1]);
        state.anim_set_flags(0, 16 | 1).unwrap();
        assert_eq!(state.animations[0].flags, 1 | 2 | 16);
        assert_eq!(frames_after(&mut state, 5), [2, 0, 1, 2, 0]);
    }

    #[test]
    fn palindrome_reverses_and_repeats_the_end_frames() {
        let mut state = animated(&[1, 1, 1]);
        state.anim_set_flags(0, 16 | 32).unwrap();
        state.anim_set_active(0, true).unwrap();
        // 0->1, 1->2, past the end (reverse, stay on 2), 2->1, 1->0,
        // before the start (reverse, stay on 0), 0->1.
        assert_eq!(frames_after(&mut state, 7), [1, 2, 2, 1, 0, 0, 1]);
        assert_eq!(state.animations[0].frame_rate, 256);
    }

    #[test]
    fn negative_frame_value_stops_and_ends_the_whole_service() {
        let overlays = (0..4).map(|_| overlay(0, 0)).collect();
        let mut state = FieldAnimation::new(&scene(
            overlays,
            vec![vec![(0, 1), (1, -1)], vec![(2, 1), (3, 1)]],
        ))
        .unwrap();
        state.anim_set_active(0, true).unwrap();
        state.anim_set_active(1, true).unwrap();
        state.tick().unwrap();
        assert_eq!(
            (state.animations[0].frame(), state.animations[1].frame()),
            (1, 1)
        );
        // Animation 0 sits on its stop frame: it stops and animation 1 is not
        // serviced this tick.
        state.tick().unwrap();
        assert_eq!(state.animations[0].flags & anim_flag::HAS_NOT_FINISHED, 0);
        assert_eq!(state.animations[0].frames[1].1, 0);
        assert_eq!(state.animations[1].frame(), 1);
        assert_eq!(state.animations[1].counter, 0);
        // Next tick animation 0 is not running, so animation 1 steps again.
        state.tick().unwrap();
        assert_eq!(state.animations[1].frame(), 0);
    }

    #[test]
    fn play_range_marks_the_last_frame_and_flips_a_backward_run() {
        let mut state = animated(&[1, 1, 1, 1]);
        state.anim_set_play_range(0, 2, 0).unwrap();
        assert_eq!(state.animations[0].frame_rate, -256);
        assert_eq!(state.animations[0].frames[0].1, -1);
        // 2 -> 1 -> 0, then the stop marker on frame 0.
        assert_eq!(frames_after(&mut state, 4), [1, 0, 0, 0]);
        assert_eq!(state.animations[0].flags & anim_flag::HAS_NOT_FINISHED, 0);
    }

    #[test]
    fn rate_scales_the_step_and_wait_edits_a_frame() {
        let mut state = animated(&[1, 1, 1, 1]);
        state.anim_set_frame_rate(0, 512).unwrap();
        state.anim_set_active(0, true).unwrap();
        assert_eq!(frames_after(&mut state, 2), [2, 0]);
        let mut state = animated(&[1, 1, 1, 1]);
        state.anim_set_frame_rate(0, 128).unwrap();
        state.anim_set_frame_wait(0, 0, 2).unwrap();
        state.anim_set_active(0, true).unwrap();
        // Frame 0 holds 2 ticks, then each step moves half a frame.
        // (4 - 1) * 128 - (4 - 1) * 256 = -384; 4 - (-384 / 256) = 5.
        assert_eq!(state.animations[0].actual_frame_count, 5);
        assert_eq!(frames_after(&mut state, 5), [0, 0, 0, 1, 1]);
    }

    #[test]
    fn show_frame_and_reset_choose_the_visible_overlay() {
        let mut state = animated(&[1, 1, 1]);
        state.anim_show_frame(0, 2).unwrap();
        assert_eq!(state.visible_frame(0), Some(2));
        assert!(!state.overlay_drawn(0));
        state.anim_show_frame(0, 255).unwrap();
        assert_eq!(state.visible_frame(0), None);
        state.anim_set_active(0, true).unwrap();
        state.anim_set_visible(0, true).unwrap();
        assert_eq!(state.visible_frame(0), Some(0));
        assert_eq!(state.animations[0].flags, anim_flag::SINGLE_FRAME);
        state.anim_set_visible(0, false).unwrap();
        assert_eq!(state.visible_frame(0), None);
    }

    #[test]
    fn animations_wait_for_their_camera() {
        let mut state = animated(&[1, 1, 1]);
        state.anim_set_active(0, true).unwrap();
        state.set_camera(1);
        assert_eq!(frames_after(&mut state, 2), [0, 0]);
        state.set_camera(0);
        assert_eq!(frames_after(&mut state, 1), [1]);
    }

    #[test]
    fn out_of_range_indexes_are_errors_not_clamps() {
        let mut state = animated(&[1, 1]);
        assert!(state.anim_set_active(5, true).is_err());
        assert!(state.overlay_set_active(-1, true).is_err());
        assert!(state.anim_set_frame_wait(0, 2, 1).is_err());
        state.anim_animate(0, 7).unwrap();
        state.anim_set_flags(0, 16).unwrap();
        assert!(state.tick().is_err());
    }

    fn scrolling(flags: u8) -> FieldAnimation {
        FieldAnimation::new(&scene(vec![overlay(0, flags)], vec![])).unwrap()
    }

    #[test]
    fn loop_scroll_adds_speed_over_256_after_wrapping_to_the_overlay_width() {
        let mut state = scrolling(2);
        state.overlay_set_loop(0, true, [128, 0]).unwrap();
        state.tick().unwrap();
        state.tick().unwrap();
        assert_eq!(state.overlays[0].cur_x, 1.0);
        // 63.75 % 64 + 1 = 64.75, then 64.75 % 64 + 1 = 1.75.
        state.overlays[0].cur_x = 63.75;
        state.overlay_set_loop(0, true, [256, 0]).unwrap();
        state.tick().unwrap();
        assert_eq!(state.overlays[0].cur_x, 64.75);
        state.tick().unwrap();
        assert_eq!(state.overlays[0].cur_x, 1.75);
        assert_eq!(state.overlays[0].cur_y, 0.0);
        state.overlay_set_loop(0, true, [32767, -64]).unwrap();
        state.tick().unwrap();
        assert_eq!(state.overlays[0].cur_x, 1.75);
        assert_eq!(state.overlays[0].cur_y, -0.25);
        state.overlay_set_loop(0, false, [256, 0]).unwrap();
        state.tick().unwrap();
        assert_eq!(state.overlays[0].cur_x, 1.75);
    }

    #[test]
    fn scroll_with_offset_moves_the_chosen_axis_only() {
        let mut state = scrolling(2);
        state
            .overlay_set_scroll_with_offset(0, true, 512, 7, false)
            .unwrap();
        assert_eq!(
            (state.overlays[0].scroll_x, state.overlays[0].scroll_y),
            (512, 7)
        );
        state.tick().unwrap();
        state.tick().unwrap();
        assert_eq!(
            (state.overlays[0].cur_x, state.overlays[0].cur_y),
            (4.0, 0.0)
        );
        state
            .overlay_set_scroll_with_offset(0, true, 256, 9, true)
            .unwrap();
        assert_eq!(
            (state.overlays[0].scroll_x, state.overlays[0].scroll_y),
            (9, 256)
        );
        state.tick().unwrap();
        assert_eq!(
            (state.overlays[0].cur_x, state.overlays[0].cur_y),
            (4.0, 1.0)
        );
    }

    #[test]
    fn parallax_follows_camera_travel_from_the_origin() {
        let mut state = scrolling(2);
        state.overlay_set_origin(0, 10.0, 4.0).unwrap();
        state.overlay_set_parallax(0, true, [128, -256]).unwrap();
        state.set_camera_position([20.0, 6.0]);
        state.tick().unwrap();
        // The origin op re-bases the camera reference after this tick's scroll.
        assert_eq!(
            (state.overlays[0].cur_x, state.overlays[0].cur_y),
            (20.0, -2.0)
        );
        state.tick().unwrap();
        assert_eq!(
            (state.overlays[0].cur_x, state.overlays[0].cur_y),
            (10.0, 4.0)
        );
        state.set_camera_position([30.0, 6.0]);
        state.tick().unwrap();
        assert_eq!(state.overlays[0].cur_x, 15.0);
    }

    #[test]
    fn disabling_parallax_keeps_only_that_bit_as_the_source_does() {
        let mut state = scrolling(2 | 4);
        state.overlay_set_parallax(0, false, [0, 0]).unwrap();
        assert_eq!(state.overlays[0].flags, 0);
        state.overlay_set_parallax(0, true, [0, 0]).unwrap();
        state.overlay_set_parallax(0, false, [0, 0]).unwrap();
        assert_eq!(state.overlays[0].flags, overlay_flag::PARALLAX);
    }

    #[test]
    fn move_clamps_to_limits_and_wraps_depth() {
        let mut state = scrolling(2);
        state.overlays[0].org_z = 65534;
        state.overlay_move(0, 5000.0, -3.0, 3).unwrap();
        let o = &state.overlays[0];
        assert_eq!((o.cur_x, o.cur_y, o.cur_z), (1000.0, -3.0, 1));
        state.overlay_move(0, -1.0, 0.0, -2).unwrap();
        let o = &state.overlays[0];
        assert_eq!((o.org_x, o.cur_y, o.cur_z), (999.0, -3.0, 65535));
    }

    #[test]
    fn timed_move_covers_its_distance_then_stops() {
        let mut state = scrolling(2);
        state.overlay_move_timed(0, 10.0, -4.0, 8, 4).unwrap();
        let mut seen = Vec::new();
        for _ in 0..6 {
            state.tick().unwrap();
            seen.push((
                state.overlays[0].cur_x,
                state.overlays[0].cur_y,
                state.overlays[0].cur_z,
            ));
        }
        assert_eq!(seen[0], (2.5, -1.0, 2));
        assert_eq!(seen[3], (10.0, -4.0, 8));
        assert_eq!(seen[5], (10.0, -4.0, 8));
    }

    #[test]
    fn viewport_ops_and_tile_placement_without_scroll() {
        let mut state = scrolling(2);
        state.overlay_define_viewport(9, [3, 4], [100, 50]);
        assert_eq!(state.viewports[0].position, [3, 4]);
        assert_eq!(state.viewports[0].size, [100, 50]);
        state.overlay_define_viewport_alpha(1, [5, 6]).unwrap();
        assert!(state.overlay_define_viewport_alpha(4, [0, 0]).is_err());
        // Not looping: the tile follows the overlay origin by its delta.
        state.overlays[0].cur_x = 5.0;
        state.overlays[0].cur_y = -2.0;
        assert_eq!(state.tile_position(0, [48, 16]).unwrap(), [53, 14]);
    }

    #[test]
    fn a_plain_overlay_may_name_a_viewport_past_the_four_windows() {
        // Shipped fields carry viewport 7 on overlays that never loop.
        let mut state = scrolling(2);
        state.overlays[0].viewport = 7;
        assert_eq!(state.tile_position(0, [16, 16]).unwrap(), [16, 16]);
        state.overlay_set_loop(0, true, [256, 0]).unwrap();
        assert!(state.tile_position(0, [16, 16]).is_err());
    }

    #[test]
    fn looping_tiles_wrap_inside_the_overlay_width() {
        // Overlay 64x32, window 320x224 at the origin, camera term dropped.
        let mut state = scrolling(2);
        state.overlay_set_loop(0, true, [256, 0]).unwrap();
        // cur_x = 10: screen 10 -> (10 - 256) % 64 + 256 = 10 - 246 % ... derive:
        // (10 - 256) = -246; -246 % 64 = -54; -54 + 256 = 202.
        state.overlays[0].cur_x = 10.0;
        // Sprite offset 0: 202 + 16 >= 64, so it wraps back by one width: 138.
        assert_eq!(state.tile_position(0, [0, 0]).unwrap(), [138, 0]);
        // Offset 48: 202 + 48 = 250; 250 + 16 >= 64 so 186.
        assert_eq!(state.tile_position(0, [48, 0]).unwrap(), [186, 0]);
    }

    #[test]
    fn a_screen_anchored_loop_follows_the_camera_and_a_plain_layer_does_not() {
        let mut state =
            FieldAnimation::new(&scene(vec![overlay(0, 2), overlay(1, 2)], vec![])).unwrap();
        state.overlay_set_loop(0, true, [256, 0]).unwrap();
        state.overlay_set_loop_type(0, true).unwrap();
        state.overlays[0].cur_x = 10.0;
        state.overlays[1].cur_x = 5.0;
        let anchored = state.tile_position(0, [0, 0]).unwrap();
        let plain = state.tile_position(1, [48, 16]).unwrap();
        // The camera 128 right and 40 down of its reference point
        // (`HalfFieldWidth - realVrp.x` = -128): an anchored layer keeps its
        // place on screen along its looping axis, so it moves with the camera
        // in background pixels; its still axis stays (the anchor and
        // `scrY` cancel).
        state.set_screen_offset([-128.0, -40.0]);
        assert_eq!(
            state.tile_position(0, [0, 0]).unwrap(),
            [anchored[0] + 128, anchored[1]]
        );
        assert_eq!(state.tile_position(1, [48, 16]).unwrap(), plain);
    }

    #[test]
    fn diagonal_scroll_shifts_the_other_axis_per_wrap() {
        let mut state = scrolling(2);
        state
            .overlay_set_scroll_with_offset(0, true, 0, 8, false)
            .unwrap();
        // is_x_offset false: scroll_y = 8. screen_x = (0 - 256) % 64 + 256 = 256;
        // screen_y = 0 + (256 * 8 / 64) % 32 = 32 % 32 = 0. Sprite x 256 + 16
        // >= 64: x = 192 and y gains -scroll_y = -8; anchor x 0.
        assert_eq!(state.tile_position(0, [0, 0]).unwrap(), [192, -8]);
    }

    fn walkmesh() -> WalkmeshAnimation {
        WalkmeshAnimation {
            triangle_flags: vec![1, 0, 1, 0, 0],
            floor_flags: vec![1, 0],
            floor_triangles: vec![vec![0, 1], vec![2, 3]],
            floors: vec![
                FloorPosition {
                    original: [100, 200, 300],
                    current: [100, 200, 300],
                };
                2
            ],
            animation_frames: vec![vec![vec![0], vec![1, 2], vec![3]]],
            animation_start_frames: vec![0x0200],
            simulations: [Simulation::new(); SIMULATION_SLOTS],
        }
    }

    #[test]
    fn floor_and_triangle_toggles_set_and_clear_the_active_bit() {
        let mut mesh = walkmesh();
        assert!(mesh.floor_set_active(1, true));
        assert!(mesh.floor_active(1));
        assert!(mesh.triangle_active(2) && mesh.triangle_active(3));
        assert!(mesh.floor_set_active(0, false));
        assert!(!mesh.triangle_active(0) && !mesh.triangle_active(1));
        assert!(!mesh.floor_set_active(2, true));
        mesh.triangle_set_active(4, true).unwrap();
        assert!(mesh.triangle_active(4));
        assert!(mesh.triangle_set_active(5, true).is_err());
    }

    #[test]
    fn showing_a_walkmesh_animation_frame_moves_the_active_triangles() {
        let mut mesh = walkmesh();
        // Triangles 0 and 2 start active; they belong to frames 0 and 1.
        // The last frame holding an active triangle is the previous frame (1).
        let shown = mesh.animation_show_frame(0, 2).unwrap();
        assert_eq!(shown.previous_frame, Some(1));
        assert!(shown.changed);
        assert_eq!(
            (0..5).map(|t| mesh.triangle_active(t)).collect::<Vec<_>>(),
            [false, false, false, true, false]
        );
        let shown = mesh.animation_show_frame(0, 2).unwrap();
        assert_eq!(shown.previous_frame, Some(2));
        assert!(!shown.changed);
        assert!(mesh.animation_show_frame(0, 3).is_none());
        assert!(mesh.animation_show_frame(1, 0).is_none());
        assert_eq!(mesh.animation_frame(0), Some(2));
    }

    #[test]
    fn simulation_walks_a_floor_axis_and_stops_at_the_end() {
        let mut mesh = walkmesh();
        mesh.simulation_set_floor(0, 1).unwrap();
        mesh.simulation_set_axis(0, 2).unwrap();
        mesh.simulation_set_delta(0, 0, 2).unwrap();
        // Starting evaluates once: frame_index 256, step 1.
        mesh.simulation_set_active(0, true).unwrap();
        assert_eq!(mesh.simulation(0).unwrap().delta_current, 1);
        assert_eq!(mesh.simulation(0).unwrap().flags, 7);
        mesh.tick().unwrap();
        assert_eq!(mesh.floor_position(1).unwrap().current, [100, 200, 299]);
        assert_eq!(mesh.simulation(0).unwrap().delta_current, 2);
        assert_eq!(mesh.simulation(0).unwrap().delta_previous, 1);
        mesh.tick().unwrap();
        assert_eq!(mesh.floor_position(1).unwrap().current, [100, 200, 298]);
        // Step 3 passes the max: back to the min, run bit (4) cleared.
        let sim = mesh.simulation(0).unwrap();
        assert_eq!((sim.delta_current, sim.flags, sim.frame_index), (0, 3, 256));
        mesh.tick().unwrap();
        assert_eq!(mesh.floor_position(1).unwrap().current, [100, 200, 298]);
    }

    #[test]
    fn simulation_ping_pong_reverses_at_the_ends() {
        let mut mesh = walkmesh();
        mesh.simulation_set_delta(1, 0, 2).unwrap();
        // Bit 16 reverses at an end, bit 8 keeps the run going afterwards.
        mesh.simulation_set_flags(1, 16 | 8 | 1).unwrap();
        assert_eq!(mesh.simulation(1).unwrap().flags, 2 | 8 | 16);
        mesh.simulation_set_active(1, true).unwrap();
        let mut deltas = vec![mesh.simulation(1).unwrap().delta_current];
        for _ in 0..6 {
            mesh.tick().unwrap();
            deltas.push(mesh.simulation(1).unwrap().delta_current);
        }
        // Start: step 1. Ticks: 2; step 3 passes the max so clamp to 2, flip
        // the rate and restart at (2 - 1) << 8; step 0 gives 0; step -1 passes
        // the min so clamp to 0 and flip back; then steps 1 and 2.
        assert_eq!(deltas, [1, 2, 2, 0, 0, 1, 2]);
        assert_eq!(mesh.simulation(1).unwrap().frame_rate, 256);
        assert_eq!(mesh.simulation(1).unwrap().flags & 4, 0);
    }

    #[test]
    fn cosine_simulation_eases_between_the_ends() {
        let mut mesh = walkmesh();
        mesh.simulation_set_delta(2, 0, 256).unwrap();
        mesh.simulation_set_algorithm(2, 1).unwrap();
        mesh.simulation_set_frame_rate(2, 25600).unwrap();
        mesh.simulation_set_active(2, true).unwrap();
        // frame_index 25600, step 100: angle = (100 << 12) / 256 + 2048 = 3648;
        // cos(3648 / 4096 turns) * 4096 + 4096 = 4096 * (1 + cos(320.625 deg)).
        let expected =
            (256.0 * (1.0 + (3648.0_f64 / 4096.0 * std::f64::consts::TAU).cos()) / 2.0) as i16;
        let got = mesh.simulation(2).unwrap().delta_current;
        assert!((got - expected).abs() <= 1, "{got} vs {expected}");
        assert!(mesh.simulation_set_axis(30, 0).is_err());
    }
}
