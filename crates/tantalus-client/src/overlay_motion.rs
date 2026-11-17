//! Moving background layers: `BGLLOOP`, `BGLPARALLAX`, `BGLSCROLLOFFSET`,
//! `BGLMOVE` and `BGLORIGIN`. The field scripts' layer calls are replayed on
//! the assets crate's `FieldAnimation` (a port of `FieldMap.SceneServiceScroll`
//! and `UpdateOverlay`), which runs once per game tick with the camera
//! position; each sprite of a scrolled layer is then placed where
//! `FieldAnimation::tile_position` puts its tile.

use crate::scene_adapter::{FieldSpriteBase, FieldSpriteIdentity};
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use tantalus_assets::pc::bgs::BgsFile;
use tantalus_assets::pc::field_animation::{FieldAnimation, overlay_flag};
use tantalus_contracts::FieldSceneDescription;
use tantalus_game::field::world::OverlayMotion;

/// Half the field frame in background pixels: sprite positions are drawn
/// centered on the frame, `FieldAnimation` plans from the top left.
const HALF_FRAME: [i32; 2] = [160, 112];

/// The largest per-axis step, in background pixels, a layer makes in one tick and is still
/// blended between ticks. A loop wrapping to its other edge, a `BGLMOVE` jump or a camera cut
/// is a step past it and is drawn at once (the same cut rule the actors and the camera follow,
/// `smooth.rs`); scroll speeds are a few pixels a tick.
pub const LAYER_SMOOTH_MAX: i32 = 16;

/// Where each scripted sprite sat on the last two ticks, as `(overlay, sprite)` to its offset
/// from the planned tile position.
type Offsets = BTreeMap<(u32, u32), [i32; 2]>;

pub struct OverlayMotionState {
    animation: FieldAnimation,
    applied: usize,
    scrolled: BTreeSet<usize>,
    /// Ticks run, and the tick `previous`/`current` were last read at.
    ticks: u64,
    sampled: u64,
    previous: Offsets,
    current: Offsets,
}

impl OverlayMotionState {
    /// `None` when the scene cannot start a `FieldAnimation`.
    pub fn new(background: &BgsFile, scene: &FieldSceneDescription) -> Option<Self> {
        let mut animation = FieldAnimation::new(background).ok()?;
        animation.set_camera(scene.camera_index as u8);
        // `FieldMap.BG_init`: the camera starts, and parallax is measured
        // from, the middle of the camera's scroll range.
        let range = crate::projection::view_range(&scene.camera);
        let center = [(range[0] + range[1]) / 2.0, -(range[2] + range[3]) / 2.0];
        animation.set_camera_position(center);
        animation.set_parallax_origin(center);
        Some(Self {
            animation,
            applied: 0,
            scrolled: BTreeSet::new(),
            ticks: 0,
            sampled: 0,
            previous: Offsets::new(),
            current: Offsets::new(),
        })
    }

    pub fn set_camera(&mut self, camera: u8) {
        self.animation.set_camera(camera);
        // A new camera lays the layers out afresh: no blend from the old one.
        self.previous.clear();
        self.current.clear();
        self.sampled = u64::MAX;
    }

    /// Whether any layer follows the camera.
    pub fn needs_camera(&self) -> bool {
        self.animation
            .overlays
            .iter()
            .any(|overlay| overlay.flags & overlay_flag::PARALLAX != 0)
    }

    /// One game tick: applies the layer calls logged since the last one,
    /// then runs the scroll service with the camera at `view` (the frame
    /// center, y up).
    pub fn tick(&mut self, log: &[OverlayMotion], view: Option<[f32; 2]>) {
        if log.len() < self.applied {
            self.applied = 0;
        }
        for motion in &log[self.applied..] {
            self.apply(*motion);
        }
        self.applied = log.len();
        self.ticks += 1;
        if let Some([x, y]) = view {
            self.animation.set_camera_position([x, -y]);
        }
        self.animation.scroll_service();
        for (index, overlay) in self.animation.overlays.iter().enumerate() {
            let scrolls = overlay.flags
                & (overlay_flag::LOOP | overlay_flag::PARALLAX | overlay_flag::SCROLL_WITH_OFFSET)
                != 0;
            if scrolls {
                self.scrolled.insert(index);
            }
        }
    }

    fn apply(&mut self, motion: OverlayMotion) {
        let animation = &mut self.animation;
        let touched = match motion {
            OverlayMotion::Loop {
                block,
                enabled,
                speed,
            } => animation
                .overlay_set_loop(block.into(), enabled, speed)
                .map(|_| block),
            OverlayMotion::Parallax {
                block,
                enabled,
                speed,
            } => animation
                .overlay_set_parallax(block.into(), enabled, speed)
                .map(|_| block),
            OverlayMotion::Origin { block, x, y } => animation
                .overlay_set_origin(block.into(), f32::from(x), f32::from(y))
                .map(|_| block),
            OverlayMotion::Move { block, offset } => animation
                .overlay_move(
                    block.into(),
                    f32::from(offset[0]),
                    f32::from(offset[1]),
                    offset[2],
                )
                .map(|_| block),
            OverlayMotion::ScrollWithOffset {
                block,
                enabled,
                speed,
                offset,
                is_x_offset,
            } => animation
                .overlay_set_scroll_with_offset(block.into(), enabled, speed, offset, is_x_offset)
                .map(|_| block),
            OverlayMotion::LoopType {
                block,
                screen_anchored,
            } => animation
                .overlay_set_loop_type(block.into(), screen_anchored)
                .map(|_| block),
            OverlayMotion::Viewport {
                viewport,
                position,
                size,
            } => {
                animation.overlay_define_viewport(viewport.into(), position, size);
                return;
            }
        };
        if let Ok(block) = touched
            && let Ok(block) = usize::try_from(block)
        {
            self.scrolled.insert(block);
        }
    }

    /// Whether any layer has been scrolled or moved by a script.
    pub fn is_idle(&self) -> bool {
        self.scrolled.is_empty()
    }

    /// Places the sprites of every scripted layer (after
    /// `apply_overlay_changes`, which this overrides for those layers), `t` of the way from
    /// the previous tick's place to this tick's (`SmoothFrameUpdater_Field` blends the
    /// layers' positions the same way). `t` of 1 draws the tick as it is.
    pub fn place_sprites(&mut self, world: &mut World, scene: &FieldSceneDescription, t: f32) {
        if self.sampled != self.ticks {
            self.sample(scene);
        }
        let mut sprites = world.query::<(&FieldSpriteIdentity, &FieldSpriteBase, &mut Transform)>();
        for (identity, base, mut transform) in sprites.iter_mut(world) {
            let key = (identity.overlay, identity.sprite);
            let Some(&now) = self.current.get(&key) else {
                continue;
            };
            let at = match self.previous.get(&key) {
                Some(&before) => blend_offset(before, now, t),
                None => [now[0] as f32, now[1] as f32],
            };
            // Pixel offsets grow down; Bevy y grows up.
            transform.translation = base.translation + Vec3::new(at[0], -at[1], 0.0);
        }
    }

    /// Reads where every sprite of a scripted layer sits after the latest tick.
    fn sample(&mut self, scene: &FieldSceneDescription) {
        self.sampled = self.ticks;
        self.previous = std::mem::take(&mut self.current);
        for overlay in &scene.overlays {
            let index = overlay.overlay_index as usize;
            if !self.scrolled.contains(&index) {
                continue;
            }
            for sprite in &overlay.sprites {
                let plan = [
                    sprite.pixel_offset[0] + HALF_FRAME[0],
                    sprite.pixel_offset[1] + HALF_FRAME[1],
                ];
                if let Ok(at) = self.animation.tile_position(index, plan) {
                    self.current.insert(
                        (overlay.overlay_index, sprite.sprite_index),
                        [at[0] - plan[0], at[1] - plan[1]],
                    );
                }
            }
        }
    }
}

/// An offset `t` of the way from `before` to `now`, except past [`LAYER_SMOOTH_MAX`] on an
/// axis (a loop's wrap, a jump, a cut), which is `now` at once.
fn blend_offset(before: [i32; 2], now: [i32; 2], t: f32) -> [f32; 2] {
    let cut = (0..2).any(|axis| (now[axis] - before[axis]).abs() > LAYER_SMOOTH_MAX);
    if cut {
        return [now[0] as f32, now[1] as f32];
    }
    [0, 1].map(|axis| before[axis] as f32 + (now[axis] - before[axis]) as f32 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_steps_blend_and_jumps_do_not() {
        assert_eq!(blend_offset([0, 0], [4, -2], 0.5), [2.0, -1.0]);
        assert_eq!(blend_offset([0, 0], [4, -2], 1.0), [4.0, -2.0]);
        // A loop wrapping from one edge to the other is not swept across.
        assert_eq!(blend_offset([-250, 0], [0, 0], 0.5), [0.0, 0.0]);
        assert_eq!(blend_offset([0, 0], [0, 17], 0.25), [0.0, 17.0]);
        assert_eq!(blend_offset([0, 0], [0, 16], 0.25), [0.0, 4.0]);
    }
}
