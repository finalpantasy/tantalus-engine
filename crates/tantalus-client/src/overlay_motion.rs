//! Moving background layers: `BGLLOOP`, `BGLPARALLAX`, `BGLSCROLLOFFSET`,
//! `BGLMOVE` and `BGLORIGIN`. The field scripts' layer calls are replayed on
//! the assets crate's `FieldAnimation` (a port of `FieldMap.SceneServiceScroll`
//! and `UpdateOverlay`), which runs once per game tick with the camera
//! position; each sprite of a scrolled layer is then placed where
//! `FieldAnimation::tile_position` puts its tile.

use crate::scene_adapter::{FieldSpriteBase, FieldSpriteIdentity};
use bevy::prelude::*;
use std::collections::BTreeSet;
use tantalus_assets::pc::bgs::BgsFile;
use tantalus_assets::pc::field_animation::{FieldAnimation, overlay_flag};
use tantalus_contracts::FieldSceneDescription;
use tantalus_game::field::world::OverlayMotion;

/// Half the field frame in background pixels: sprite positions are drawn
/// centered on the frame, `FieldAnimation` plans from the top left.
const HALF_FRAME: [i32; 2] = [160, 112];

pub struct OverlayMotionState {
    animation: FieldAnimation,
    applied: usize,
    scrolled: BTreeSet<usize>,
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
        })
    }

    pub fn set_camera(&mut self, camera: u8) {
        self.animation.set_camera(camera);
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
    /// `apply_overlay_changes`, which this overrides for those layers).
    pub fn place_sprites(&self, world: &mut World, scene: &FieldSceneDescription) {
        let mut sprites = world.query::<(&FieldSpriteIdentity, &FieldSpriteBase, &mut Transform)>();
        for (identity, base, mut transform) in sprites.iter_mut(world) {
            let overlay = identity.overlay as usize;
            if !self.scrolled.contains(&overlay) {
                continue;
            }
            let Some(sprite) = scene
                .overlays
                .iter()
                .find(|candidate| candidate.overlay_index == identity.overlay)
                .and_then(|candidate| {
                    candidate
                        .sprites
                        .iter()
                        .find(|sprite| sprite.sprite_index == identity.sprite)
                })
            else {
                continue;
            };
            let plan = [
                sprite.pixel_offset[0] + HALF_FRAME[0],
                sprite.pixel_offset[1] + HALF_FRAME[1],
            ];
            let Ok(at) = self.animation.tile_position(overlay, plan) else {
                continue;
            };
            // Pixel offsets grow down; Bevy y grows up.
            transform.translation = base.translation
                + Vec3::new((at[0] - plan[0]) as f32, -((at[1] - plan[1]) as f32), 0.0);
        }
    }
}
