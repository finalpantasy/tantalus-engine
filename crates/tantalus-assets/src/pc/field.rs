use crate::install::ImportError;
use crate::pc::bgi::BgiFile;
use crate::pc::bgs::BgsFile;
use tantalus_contracts::{
    ContentId, FieldOverlayPresentation, FieldSceneDescription, FieldSpritePresentation,
    LegacyFieldCamera, Point3,
};

pub struct FieldMapper;

impl FieldMapper {
    pub fn map(
        scene: &BgsFile,
        navigation: &BgiFile,
        atlas: ContentId,
        camera_index: u32,
    ) -> Result<FieldSceneDescription, ImportError> {
        let camera = scene.cameras.get(camera_index as usize).ok_or_else(|| {
            ImportError::invalid(
                "field-mapper",
                format!("invalid field camera {camera_index}"),
            )
        })?;
        let mut overlays = Vec::with_capacity(scene.overlays.len());
        for (overlay_index, overlay) in scene.overlays.iter().enumerate() {
            let mut sprites = Vec::with_capacity(overlay.sprites.len());
            for (sprite_index, sprite) in overlay.sprites.iter().enumerate() {
                let right = sprite.u as u32 + sprite.width as u32;
                let bottom = sprite.v as u32 + sprite.height as u32;
                let depth = (overlay.current_z as u32)
                    .checked_shl(scene.depth_bit_shift as u32)
                    .and_then(|depth| depth.checked_add(sprite.depth as u32))
                    .ok_or_else(|| ImportError::invalid("field-mapper", "sprite depth overflow"))?;
                sprites.push(FieldSpritePresentation {
                    overlay_index: overlay_index as u32,
                    sprite_index: sprite_index as u32,
                    atlas: atlas.clone(),
                    atlas_rect: [sprite.u as u32, sprite.v as u32, right, bottom],
                    pixel_offset: [
                        overlay.current[0] as i32 + sprite.offset_x as i32,
                        overlay.current[1] as i32 + sprite.offset_y as i32,
                    ],
                    depth,
                    alpha: sprite.alpha,
                    transparency: sprite.transparency,
                    active: overlay.flags & 2 != 0,
                    screen_anchored: overlay.flags & 1 != 0,
                });
            }
            overlays.push(FieldOverlayPresentation {
                overlay_index: overlay_index as u32,
                camera_index: overlay.camera,
                flags: overlay.flags,
                pixel_size: [overlay.width, overlay.height],
                current_offset: overlay.current,
                sprites,
            });
        }
        Ok(FieldSceneDescription {
            camera_index,
            camera: LegacyFieldCamera {
                projection: camera.projection,
                rotation: camera
                    .rotation
                    .concat()
                    .try_into()
                    .expect("3x3 camera matrix"),
                translation: Point3 {
                    x: camera.translation[0],
                    y: camera.translation[1],
                    z: camera.translation[2],
                },
                center_offset: camera.center_offset,
                viewport_x: camera.viewport_x,
                viewport_y: camera.viewport_y,
                depth_offset: camera.depth_offset,
            },
            actor_anchor: Point3 {
                x: navigation.character[0] as i32,
                y: navigation.character[1] as i32,
                z: navigation.character[2] as i32,
            },
            navigation_triangles: navigation.triangles.len() as u32,
            overlays,
        })
    }
}
