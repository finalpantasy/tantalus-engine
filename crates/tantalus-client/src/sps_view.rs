//! Field SPS effects on screen: small sprite strips (flames, sparkles, smoke) the field scripts
//! place in the world (`SPSEffect._GenerateSPSMesh`, field mode).
//!
//! Each loaded effect is a mesh of quads: the effect's world position goes through the field
//! camera (`PSX.CalculateGTE_RTPT_POS`), its scale is divided by the view depth, and each
//! sprite is offset from that center by its own `sbyte << 2` units. The blend modes are
//! `PSX/FieldSPS_Abr_0`..`_3` and `_None` (`Shaders/PSX/FieldSPS_*.txt`).

use crate::actor_models::ActorModels;
use crate::projection::{layer_z, project_field_point_depth};
use crate::track::TrackFrame;
use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path};
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d,
    RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension,
    TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin};
use std::collections::HashMap;
use tantalus_assets::pc::field_sps::{FieldSpsLibrary, SpsFile};
use tantalus_contracts::LegacyFieldCamera;
use tantalus_game::field::sps::SpsSlot;

/// Registers the shader and the material.
pub struct SpsPlugin;

impl Plugin for SpsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/sps.wgsl");
        app.add_plugins(Material2dPlugin::<SpsMaterial>::default());
        app.init_resource::<SpsCache>();
    }
}

#[derive(Debug, Clone, Copy, Default, ShaderType)]
pub struct SpsParams {
    pub mode: u32,
}

/// One effect page with the blend mode its slot chose.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(SpsKey)]
pub struct SpsMaterial {
    #[uniform(0)]
    pub params: SpsParams,
    #[texture(1)]
    #[sampler(2)]
    pub page: Handle<Image>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpsKey {
    mode: u8,
}

impl From<&SpsMaterial> for SpsKey {
    fn from(material: &SpsMaterial) -> Self {
        Self {
            mode: material.params.mode as u8,
        }
    }
}

/// The PSX blend of each `FieldSPS_Abr_*` shader: 1 and 3 add, 2 subtracts, 0 blends by the
/// texel alpha and 4 (`_None`) draws premultiplied.
pub fn blend_state(mode: u8) -> BlendState {
    let color = match mode {
        1 | 3 => BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        },
        2 => BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::ReverseSubtract,
        },
        0 => BlendComponent {
            src_factor: BlendFactor::SrcAlpha,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        },
        _ => BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        },
    };
    BlendState {
        color,
        alpha: BlendComponent {
            src_factor: BlendFactor::Zero,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        },
    }
}

impl Material2d for SpsMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("shaders/sps.wgsl")).with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(blend_state(key.bind_group_data.mode));
            }
        }
        Ok(())
    }
}

/// Texture pages and materials made so far, by field name, file and blend mode.
#[derive(Resource, Default)]
pub struct SpsCache {
    pages: HashMap<(String, i32), Handle<Image>>,
    materials: HashMap<(String, i32, u8), Handle<SpsMaterial>>,
}

/// Marks the effects drawn this tick.
#[derive(Component)]
pub struct SpsSprite;

/// `SPSConst.ABR_*` to the blend mode the material draws (`min(abr, 4)`).
fn blend_mode(abr: u8) -> u8 {
    abr.min(4)
}

/// The vertex color of a sprite: the file's byte times `fade` over 256 (kept under 0x7FFF),
/// over 127, or over 255 for the save spheres Memoria brightens (`_GenerateSPSPrims`).
pub fn sprite_color(color: [u8; 3], fade: i32, field: i32, id: i32) -> [f32; 3] {
    let bright = matches!(
        (field, id),
        (2901, 644 | 736) | (2913, 646 | 737) | (2925, 990 | 988)
    );
    let base = if bright { 255.0 } else { 127.0 };
    color.map(|channel| {
        let value = if fade >= 0 {
            ((u32::from(channel) * fade as u32) >> 8).min(0x7FFF)
        } else {
            u32::from(channel)
        };
        value as f32 / base
    })
}

/// The mesh of one slot at its frame: four vertices per sprite, centered on `center`.
/// `scale` is the final size factor (script scale over 4096 times the projection),
/// `angle` the turn about the center in radians.
pub fn build_sprites(
    file: &SpsFile,
    frame: usize,
    center: [f32; 2],
    scale: f32,
    angle: f32,
    colors: impl Fn([u8; 3]) -> [f32; 3],
) -> Option<Mesh> {
    let prims = file.frames.get(frame)?;
    if prims.is_empty() {
        return None;
    }
    let (texel_w, texel_h) = file.texel_size();
    // `uvShrink`: field effects sample half a texel in from each edge.
    let (w, h) = (texel_w as f32, texel_h as f32);
    let (sin, cos) = angle.sin_cos();
    let mut positions = Vec::with_capacity(prims.len() * 4);
    let mut uvs = Vec::with_capacity(prims.len() * 4);
    let mut colored = Vec::with_capacity(prims.len() * 4);
    let mut indices = Vec::with_capacity(prims.len() * 6);
    for (index, prim) in prims.iter().enumerate() {
        let (half_w, half_h) = (file.half_width as f32, file.half_height as f32);
        let (x, y) = (prim.x as f32, prim.y as f32);
        // Corners in the order the reference makes them: (min, min), (max, min), (min, max),
        // (max, max), with the effect's y running down the screen.
        let corners = [
            (x - half_w, y - half_h, 0.5, 0.5),
            (x + half_w, y - half_h, w - 0.5, 0.5),
            (x - half_w, y + half_h, 0.5, h - 0.5),
            (x + half_w, y + half_h, w - 0.5, h - 0.5),
        ];
        let [r, g, b] = colors(prim.color);
        // Later sprites draw over earlier ones: a hair nearer for each.
        let z = index as f32 * 0.0005;
        for (cx, cy, du, dv) in corners {
            let (sx, sy) = (cx * scale, -cy * scale);
            positions.push([
                center[0] + sx * cos - sy * sin,
                center[1] + sx * sin + sy * cos,
                z,
            ]);
            uvs.push([
                (f32::from(prim.u) + du) / 256.0,
                (f32::from(prim.v) + dv) / 256.0,
            ]);
            colored.push([r, g, b, 1.0]);
        }
        let base = (index * 4) as u32;
        indices.extend([base, base + 1, base + 2, base + 1, base + 3, base + 2]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colored);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}

fn page_image(file: &SpsFile, vram: &tantalus_assets::pc::field_sps::Vram) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: 256,
            height: 256,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        file.page_rgba(vram),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// What a sync needs to know about the running game.
pub struct SpsSource<'a> {
    pub slots: &'a [SpsSlot],
    /// The current field's id and its background name.
    pub field: (i32, &'a str),
    /// A field id's background name (`EventEngineUtils.eventIDToFBGID`).
    pub background_of: &'a dyn Fn(i32) -> Option<String>,
    /// An object's actor id by script uid.
    pub actor_of: &'a dyn Fn(i32) -> Option<u64>,
}

/// Replaces last tick's effects with the ones `source` shows.
pub fn sync_sps(
    world: &mut World,
    source: &SpsSource<'_>,
    camera: &LegacyFieldCamera,
    frame: &TrackFrame,
    models: Option<&ActorModels>,
    library: &FieldSpsLibrary,
) -> usize {
    let mut old = world.query_filtered::<(Entity, &Mesh2d), With<SpsSprite>>();
    let old: Vec<(Entity, AssetId<Mesh>)> = old
        .iter(world)
        .map(|(entity, mesh)| (entity, mesh.0.id()))
        .collect();
    for (entity, mesh) in old {
        world.entity_mut(entity).despawn();
        world.resource_mut::<Assets<Mesh>>().remove(mesh);
    }
    let mut drawn = 0;
    for slot in source.slots {
        let Some(frame_index) = slot.drawn_frame() else {
            continue;
        };
        let origin = if slot.origin == source.field.0 {
            Some(source.field.1.to_owned())
        } else {
            (source.background_of)(slot.origin)
        };
        let Some(origin) = origin else { continue };
        let Some(field) = library.field(&origin) else {
            continue;
        };
        let Some(file) = field.files.get(&slot.id) else {
            continue;
        };
        // A slot following a bone takes its position every tick (`FieldSPSSystem.GenerateSPS`).
        let mut pos = slot.pos;
        if slot.char_uid >= 0 {
            let bone = (source.actor_of)(slot.char_uid).and_then(|actor| {
                let at = frame.actors.iter().find(|(id, _)| *id == actor)?.1;
                models?.bone_position(frame.poses.get(&actor)?, at, slot.bone)
            });
            // Without the bone the reference leaves the position where the script put it.
            if let Some(bone) = bone {
                pos = bone;
            }
        }
        let at = [
            pos[0] + slot.pos_offset[0],
            pos[1] + slot.pos_offset[1],
            pos[2] + slot.pos_offset[2],
        ];
        let Some(([x, y], depth)) = project_field_point_depth(camera, at) else {
            continue;
        };
        // `localPos.z < 0`: behind the camera.
        if depth <= 0.0 {
            continue;
        }
        let scale = slot.scale as f32 / 4096.0 * f32::from(camera.projection) / depth;
        // A flame on an actor's hand sits at the actor's own depth; the actor's z carries up to
        // 0.01 for its raster, so the effect goes just above that (still under any layer one
        // depth unit nearer, 0.04 up).
        let z =
            layer_z((depth / 4.0).trunc() + camera.depth_offset as f32 + slot.depth_offset as f32)
                + 0.015;
        let (field_id, id, fade) = (source.field.0, slot.id, slot.fade);
        let Some(mesh) = build_sprites(
            file,
            frame_index,
            [x, y],
            scale,
            (-slot.rot[2]).to_radians(),
            |color| sprite_color(color, fade, field_id, id),
        ) else {
            continue;
        };
        let mode = blend_mode(slot.abr);
        let key = (origin.clone(), slot.id, mode);
        let material = match world.resource::<SpsCache>().materials.get(&key) {
            Some(material) => material.clone(),
            None => {
                let page = match world
                    .resource::<SpsCache>()
                    .pages
                    .get(&(origin.clone(), slot.id))
                {
                    Some(page) => page.clone(),
                    None => {
                        let page = world
                            .resource_mut::<Assets<Image>>()
                            .add(page_image(file, &field.vram));
                        world
                            .resource_mut::<SpsCache>()
                            .pages
                            .insert((origin.clone(), slot.id), page.clone());
                        page
                    }
                };
                let material = world
                    .resource_mut::<Assets<SpsMaterial>>()
                    .add(SpsMaterial {
                        params: SpsParams {
                            mode: u32::from(mode),
                        },
                        page,
                    });
                world
                    .resource_mut::<SpsCache>()
                    .materials
                    .insert(key, material.clone());
                material
            }
        };
        let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
        world.spawn((
            Mesh2d(mesh),
            MeshMaterial2d(material),
            Transform::from_xyz(0.0, 0.0, z),
            SpsSprite,
        ));
        drawn += 1;
    }
    drawn
}

/// The library of the field bundles under `streaming_assets` (`p0data11.bin`-`p0data19.bin`).
pub fn library(streaming_assets: &std::path::Path) -> FieldSpsLibrary {
    FieldSpsLibrary::new(
        (11..=19)
            .map(|n| streaming_assets.join(format!("p0data{n}.bin")))
            .collect(),
    )
}

/// [`sync_sps`] for the running game; returns how many effects it drew.
pub fn sync_game_sps(
    world: &mut World,
    game: &tantalus_tools::play::PlayGame,
    camera: &LegacyFieldCamera,
    frame: &TrackFrame,
    models: Option<&ActorModels>,
    library: &FieldSpsLibrary,
) -> usize {
    let (Some(map), Some(background)) = (game.map(), game.background()) else {
        return 0;
    };
    let state = game.world();
    let background_of = |field: i32| game.background_of(field).map(str::to_owned);
    let actor_of = |uid: i32| state.resolve_uid(uid).map(|actor| actor.get());
    sync_sps(
        world,
        &SpsSource {
            slots: &state.sps.slots,
            field: (map, background),
            background_of: &background_of,
            actor_of: &actor_of,
        },
        camera,
        frame,
        models,
        library,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_assets::pc::field_sps::SpsPrim;

    fn file() -> SpsFile {
        SpsFile {
            frame_count: 1,
            tpage: 0,
            clut: 0,
            half_height: 8,
            half_width: 16,
            frames: vec![vec![SpsPrim {
                x: 4,
                y: -8,
                u: 32,
                v: 64,
                color: [128, 64, 0],
            }]],
        }
    }

    #[test]
    fn sprite_color_follows_the_fade_formula() {
        // `r * fade >> 8` over 127: a byte of 254 at the default fade of 128 is 127 / 127.
        assert_eq!(
            sprite_color([254, 0, 127], 128, 50, 42),
            [1.0, 0.0, 63.0 / 127.0]
        );
        assert_eq!(sprite_color([255, 0, 0], 128, 2901, 644)[0], 127.0 / 255.0);
        assert_eq!(
            sprite_color([10, 20, 30], -1, 50, 1),
            [10.0 / 127.0, 20.0 / 127.0, 30.0 / 127.0]
        );
    }

    #[test]
    fn a_sprite_is_a_quad_around_its_center_with_texels_shrunk_half_a_pixel() {
        let mesh = build_sprites(&file(), 0, [100.0, 50.0], 0.5, 0.0, |_| [1.0; 3]).unwrap();
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|values| values.as_float3())
            .unwrap();
        // (x - 16, y - 8) = (-12, -16), scaled by 0.5 with y flipped: (94, 58).
        assert_eq!(positions[0][0], 100.0 + -12.0 * 0.5);
        assert_eq!(positions[0][1], 50.0 + 16.0 * 0.5);
        assert_eq!(positions[3][0], 100.0 + 20.0 * 0.5);
        assert_eq!(positions[3][1], 50.0 - 0.0 * 0.5);
        let uvs: Vec<[f32; 2]> = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) => uvs.clone(),
            other => panic!("uvs: {other:?}"),
        };
        // Texels are (w >> 1, h >> 1) = (8, 4): corners at 0.5 and size - 0.5.
        assert_eq!(uvs[0], [32.5 / 256.0, 64.5 / 256.0]);
        assert_eq!(uvs[3], [(32.0 + 7.5) / 256.0, (64.0 + 3.5) / 256.0]);
    }

    #[test]
    fn blend_modes_match_the_shaders() {
        assert_eq!(
            blend_state(2).color.operation,
            BlendOperation::ReverseSubtract
        );
        assert_eq!(blend_state(1), blend_state(3));
        assert_eq!(blend_state(0).color.src_factor, BlendFactor::SrcAlpha);
        assert_eq!(
            blend_state(4).color.dst_factor,
            BlendFactor::OneMinusSrcAlpha
        );
        assert_eq!(blend_mode(15), 4);
        assert_eq!(blend_mode(2), 2);
    }
}
