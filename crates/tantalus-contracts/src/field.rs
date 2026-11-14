use crate::{ContentId, Point3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyFieldCamera {
    pub projection: u16,
    pub rotation: [i16; 9],
    pub translation: Point3,
    pub center_offset: [i16; 2],
    pub viewport_x: [i16; 2],
    pub viewport_y: [i16; 2],
    pub depth_offset: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSpritePresentation {
    pub overlay_index: u32,
    pub sprite_index: u32,
    pub atlas: ContentId,
    pub atlas_rect: [u32; 4],
    pub pixel_offset: [i32; 2],
    pub depth: u32,
    pub alpha: u8,
    pub transparency: u8,
    pub active: bool,
    pub screen_anchored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldOverlayPresentation {
    pub overlay_index: u32,
    pub camera_index: u8,
    pub flags: u8,
    pub pixel_size: [u16; 2],
    pub current_offset: [i16; 2],
    pub sprites: Vec<FieldSpritePresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSceneDescription {
    pub camera_index: u32,
    pub camera: LegacyFieldCamera,
    pub actor_anchor: Point3,
    pub navigation_triangles: u32,
    pub overlays: Vec<FieldOverlayPresentation>,
}
