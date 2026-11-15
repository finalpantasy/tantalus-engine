use crate::install::ImportError;
use crate::pc::bgi::BgiFile;
use crate::pc::bgs::BgsFile;
use tantalus_contracts::{
    ContentId, FieldOverlayPresentation, FieldSceneDescription, FieldSpritePresentation,
    LegacyFieldCamera, Point3,
};

pub struct FieldMapper;

#[derive(Clone, Copy)]
struct PaddedAtlas {
    width: u32,
    height: u32,
    tile_size: u32,
    padding: u32,
}

impl FieldMapper {
    pub fn map(
        scene: &BgsFile,
        navigation: &BgiFile,
        atlas: ContentId,
        camera_index: u32,
    ) -> Result<FieldSceneDescription, ImportError> {
        Self::map_with_layout(scene, navigation, atlas, camera_index, None)
    }

    pub fn map_upscaled(
        scene: &BgsFile,
        navigation: &BgiFile,
        atlas: ContentId,
        camera_index: u32,
        atlas_size: [u32; 2],
        tile_size: u32,
        padding: u32,
    ) -> Result<FieldSceneDescription, ImportError> {
        if tile_size == 0 || atlas_size.contains(&0) {
            return Err(ImportError::invalid(
                "field-mapper",
                "upscaled atlas dimensions and tile size must be positive",
            ));
        }
        Self::map_with_layout(
            scene,
            navigation,
            atlas,
            camera_index,
            Some(PaddedAtlas {
                width: atlas_size[0],
                height: atlas_size[1],
                tile_size,
                padding,
            }),
        )
    }

    fn map_with_layout(
        scene: &BgsFile,
        navigation: &BgiFile,
        atlas: ContentId,
        camera_index: u32,
        layout: Option<PaddedAtlas>,
    ) -> Result<FieldSceneDescription, ImportError> {
        let camera = scene.cameras.get(camera_index as usize).ok_or_else(|| {
            ImportError::invalid(
                "field-mapper",
                format!("invalid field camera {camera_index}"),
            )
        })?;
        if let Some(layout) = layout {
            let stride = layout.tile_size + layout.padding * 2;
            let columns = (layout.width / stride) as usize;
            let rows = (layout.height / stride) as usize;
            let sprite_count = scene
                .overlays
                .iter()
                .map(|overlay| overlay.sprites.len())
                .sum::<usize>();
            if columns == 0 || rows == 0 || sprite_count > columns * rows {
                return Err(ImportError::invalid(
                    "field-mapper",
                    format!("upscaled atlas cannot hold {sprite_count} padded tiles"),
                ));
            }
        }

        let mut sprite_ordinal = 0_usize;
        let mut overlays = Vec::with_capacity(scene.overlays.len());
        for (overlay_index, overlay) in scene.overlays.iter().enumerate() {
            let mut sprites = Vec::with_capacity(overlay.sprites.len());
            for (sprite_index, sprite) in overlay.sprites.iter().enumerate() {
                let atlas_rect = match layout {
                    Some(layout) => padded_rect(layout, sprite_ordinal),
                    None => [
                        sprite.u as u32,
                        sprite.v as u32,
                        sprite.u as u32 + sprite.width as u32,
                        sprite.v as u32 + sprite.height as u32,
                    ],
                };
                sprite_ordinal += 1;
                let depth = (overlay.current_z as u32)
                    .checked_shl(scene.depth_bit_shift as u32)
                    .and_then(|depth| depth.checked_add(sprite.depth as u32))
                    .ok_or_else(|| ImportError::invalid("field-mapper", "sprite depth overflow"))?;
                sprites.push(FieldSpritePresentation {
                    overlay_index: overlay_index as u32,
                    sprite_index: sprite_index as u32,
                    atlas: atlas.clone(),
                    atlas_rect,
                    pixel_size: [sprite.width as u32, sprite.height as u32],
                    pixel_offset: [
                        scene.current[0] as i32 - 160
                            + overlay.current[0] as i32
                            + sprite.offset_x as i32,
                        scene.current[1] as i32 - 112
                            + overlay.current[1] as i32
                            + sprite.offset_y as i32,
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
                size: [camera.width as u16, camera.height as u16],
                viewport_x: camera.viewport_x,
                viewport_y: camera.viewport_y,
                depth_offset: camera.depth_offset,
            },
            actor_anchor: Point3 {
                x: navigation.character[0] as i32,
                y: navigation.character[1] as i32,
                z: navigation.character[2] as i32,
            },
            navigation: map_navigation(navigation)?,
            overlays,
        })
    }
}

fn padded_rect(layout: PaddedAtlas, ordinal: usize) -> [u32; 4] {
    let stride = layout.tile_size + layout.padding * 2;
    let columns = (layout.width / stride) as usize;
    let x = layout.padding + (ordinal % columns) as u32 * stride;
    let y = layout.padding + (ordinal / columns) as u32 * stride;
    [x, y, x + layout.tile_size, y + layout.tile_size]
}

fn map_navigation(value: &BgiFile) -> Result<tantalus_contracts::FieldNavigationMesh, ImportError> {
    let mut triangles = Vec::with_capacity(value.triangles.len());
    for triangle in &value.triangles {
        let floor = value
            .floors
            .iter()
            .find(|floor| floor.index as i16 == triangle.floor)
            .ok_or_else(|| {
                ImportError::invalid(
                    "field-mapper",
                    format!(
                        "navigation triangle references missing floor {}",
                        triangle.floor
                    ),
                )
            })?;
        let offset = Point3 {
            x: value.origin[0] as i32 + floor.bounds[0][0] as i32,
            y: value.origin[1] as i32 + floor.bounds[0][1] as i32,
            z: value.origin[2] as i32 + floor.bounds[0][2] as i32,
        };
        triangles.push(tantalus_contracts::NavigationTriangle {
            vertices: triangle.vertices.map(|vertex| vertex as u32),
            floor: triangle.floor,
            offset,
        });
    }
    Ok(tantalus_contracts::FieldNavigationMesh {
        vertices: value
            .vertices
            .iter()
            .map(|vertex| Point3 {
                x: vertex[0] as i32,
                y: vertex[1] as i32,
                z: vertex[2] as i32,
            })
            .collect(),
        triangles,
        active_floor: value.active_floor,
        active_triangle: value.active_triangle as i32,
    })
}
