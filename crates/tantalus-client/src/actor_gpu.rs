//! Field actors drawn on the GPU: each actor's projected triangles become
//! one mesh (sorted far to near, since the field's 2D pass has no depth
//! buffer) over a texture atlas, shaded by `shaders/actor_gpu.wgsl`.
//! The CPU rasterizer ([`crate::actor_models::rasterize`]) stays for
//! captures made without [`ActorGpuPlugin`].

use std::collections::HashMap;

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path};
use bevy::image::ImageSampler;
use bevy::mesh::PrimitiveTopology;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin};

use crate::actor_models::{ShadowQuad, Texels, Triangle};

/// Registers the shader, the material and the atlas cache.
pub struct ActorGpuPlugin;

impl Plugin for ActorGpuPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/actor_gpu.wgsl");
        embedded_asset!(app, "shaders/actor_shadow.wgsl");
        app.add_plugins(Material2dPlugin::<ActorGpuMaterial>::default());
        app.add_plugins(Material2dPlugin::<ActorShadowMaterial>::default());
        app.init_resource::<ActorGpuCache>();
    }
}

/// An atlas of the textures one actor draws.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ActorGpuMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub atlas: Handle<Image>,
}

impl Material2d for ActorGpuMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("shaders/actor_gpu.wgsl"))
                .with_source("embedded"),
        )
    }

    // Sorted with the background sprites by z in the transparent phase.
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

/// The shadow texture, drawn as a reverse-subtract quad per actor
/// (`PSX/FieldMapActorShadow`: `Blend One One`, `BlendOp RevSub`).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ActorShadowMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub plate: Handle<Image>,
}

impl Material2d for ActorShadowMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("shaders/actor_shadow.wgsl"))
                .with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(crate::psx_blend::blend_state(2));
            }
        }
        Ok(())
    }
}

/// A texture as the atlas key: where its texels live and how big it is.
type TextureKey = (usize, usize, usize, u64);

struct Atlas {
    material: Handle<ActorGpuMaterial>,
    /// Per texture: (u0, v0, du, dv) in the atlas.
    rects: HashMap<TextureKey, [f32; 4]>,
}

/// Atlases built so far, by the set of textures they hold.
#[derive(Resource, Default)]
pub struct ActorGpuCache {
    atlases: HashMap<Vec<TextureKey>, Atlas>,
    /// The shadow material and the plate size it was made from.
    shadow: Option<((usize, usize), Handle<ActorShadowMaterial>)>,
}

/// An actor ready to spawn.
pub struct GpuActor {
    pub mesh: Handle<Mesh>,
    pub material: Handle<ActorGpuMaterial>,
    /// Nearest view depth, for ordering actors.
    pub depth: f32,
    /// Field-space bounds (x right, y up): min x, min y, max x, max y.
    pub bounds: [f32; 4],
}

/// The texel address and size, plus a few sampled bytes so that a new
/// texture loaded at a freed address is not mistaken for the old one.
fn key_of(texels: &Texels<'_>) -> TextureKey {
    let step = (texels.rgba.len() / 61).max(1);
    let sample = texels.rgba.iter().step_by(step).fold(0u64, |hash, byte| {
        hash.wrapping_mul(1_099_511_628_211) ^ u64::from(*byte)
    });
    (
        texels.rgba.as_ptr() as usize,
        texels.width,
        texels.height,
        sample,
    )
}

/// The mesh (two triangles) and material for one shadow quad, over
/// `plate` (width, height, RGBA rows top-down). The shared material holds
/// the plate; the strength rides in the vertex color.
pub fn build_shadow(
    world: &mut World,
    plate: &crate::field_light::ShadowPlate,
    quad: &ShadowQuad,
) -> (Handle<Mesh>, Handle<ActorShadowMaterial>) {
    let size = (plate.0, plate.1);
    let known = world
        .resource::<ActorGpuCache>()
        .shadow
        .as_ref()
        .filter(|(known, _)| *known == size)
        .map(|(_, material)| material.clone());
    let material = known.unwrap_or_else(|| {
        let mut image = Image::new(
            Extent3d {
                width: plate.0 as u32,
                height: plate.1 as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            plate.2.clone(),
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::default(),
        );
        image.sampler = ImageSampler::nearest();
        let image = world.resource_mut::<Assets<Image>>().add(image);
        let material = world
            .resource_mut::<Assets<ActorShadowMaterial>>()
            .add(ActorShadowMaterial { plate: image });
        world.resource_mut::<ActorGpuCache>().shadow = Some((size, material.clone()));
        material
    });
    let color = [
        quad.strength,
        0.0,
        0.0,
        crate::field_light::SHADOW_VERTEX_ALPHA,
    ];
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        quad.corners
            .iter()
            .map(|vertex| [vertex.xy[0], vertex.xy[1], 0.0])
            .collect::<Vec<_>>(),
    );
    // The plate's rows run top-down: v = 0 is the last row.
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        quad.corners
            .iter()
            .map(|vertex| [vertex.uv[0], 1.0 - vertex.uv[1]])
            .collect::<Vec<_>>(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![color; 4]);
    mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
    (mesh, material)
}

/// Builds the mesh and atlas for `triangles`. `None` when there is nothing to draw.
pub fn build_actor(world: &mut World, triangles: &[Triangle<'_>]) -> Option<GpuActor> {
    if triangles.is_empty() {
        return None;
    }
    let mut textures: Vec<(TextureKey, Texels<'_>)> = Vec::new();
    for (_, texels) in triangles {
        if let Some(texels) = texels {
            let key = key_of(texels);
            if !textures.iter().any(|(known, _)| *known == key) {
                textures.push((key, *texels));
            }
        }
    }
    textures.sort_by_key(|(key, _)| *key);
    let set: Vec<TextureKey> = textures.iter().map(|(key, _)| *key).collect();
    if !world.resource::<ActorGpuCache>().atlases.contains_key(&set) {
        let atlas = build_atlas(world, &textures);
        world
            .resource_mut::<ActorGpuCache>()
            .atlases
            .insert(set.clone(), atlas);
    }
    let cache = world.resource::<ActorGpuCache>();
    let atlas = &cache.atlases[&set];
    let material = atlas.material.clone();
    // Far to near, so nearer triangles are drawn over farther ones.
    let mut order: Vec<usize> = (0..triangles.len()).collect();
    let depth_of = |index: usize| {
        let [a, b, c] = &triangles[index].0;
        (a.depth + b.depth + c.depth) / 3.0
    };
    order.sort_by(|a, b| depth_of(*b).total_cmp(&depth_of(*a)));
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(triangles.len() * 3);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(triangles.len() * 3);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(triangles.len() * 3);
    let mut nearest = f32::INFINITY;
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for index in order {
        let (vertices, texels) = &triangles[index];
        let rect = texels
            .as_ref()
            .and_then(|texels| atlas.rects.get(&key_of(texels)))
            .copied()
            .unwrap_or([0.0; 4]);
        // Repeat: shift the triangle's coordinates into the first tile.
        let shift = [0, 1].map(|axis| {
            -vertices
                .iter()
                .map(|vertex| vertex.uv[axis])
                .fold(f32::INFINITY, f32::min)
                .floor()
        });
        for vertex in vertices {
            nearest = nearest.min(vertex.depth);
            bounds[0] = bounds[0].min(vertex.xy[0]);
            bounds[1] = bounds[1].min(vertex.xy[1]);
            bounds[2] = bounds[2].max(vertex.xy[0]);
            bounds[3] = bounds[3].max(vertex.xy[1]);
            positions.push([vertex.xy[0], vertex.xy[1], 0.0]);
            // The texture's rows run top-down: v = 0 is the last row.
            let u = vertex.uv[0] + shift[0];
            let v = 1.0 - (vertex.uv[1] + shift[1]);
            uvs.push([rect[0] + u * rect[2], rect[1] + v * rect[3]]);
            colors.push(if texels.is_some() {
                vertex.color
            } else {
                // Untextured: the reference's gray texel.
                let gray = 160.0 / 255.0;
                [
                    gray * vertex.color[0],
                    gray * vertex.color[1],
                    gray * vertex.color[2],
                    vertex.color[3],
                ]
            });
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
    Some(GpuActor {
        mesh,
        material,
        depth: nearest,
        bounds,
    })
}

/// Stacks `textures` into one image, each as a block of rows. The first
/// texel is opaque white, for triangles with no texture.
fn build_atlas(world: &mut World, textures: &[(TextureKey, Texels<'_>)]) -> Atlas {
    let width = textures
        .iter()
        .map(|(_, texels)| texels.width)
        .max()
        .unwrap_or(1);
    let height: usize = 1 + textures
        .iter()
        .map(|(_, texels)| texels.height)
        .sum::<usize>();
    let mut data = vec![0u8; width * height * 4];
    data[..4].copy_from_slice(&[255, 255, 255, 255]);
    let mut rects = HashMap::new();
    let mut row = 1;
    for (key, texels) in textures {
        for y in 0..texels.height {
            let from = y * texels.width * 4;
            let to = (row + y) * width * 4;
            data[to..to + texels.width * 4]
                .copy_from_slice(&texels.rgba[from..from + texels.width * 4]);
        }
        rects.insert(
            *key,
            [
                0.0,
                row as f32 / height as f32,
                texels.width as f32 / width as f32,
                texels.height as f32 / height as f32,
            ],
        );
        row += texels.height;
    }
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    let image = world.resource_mut::<Assets<Image>>().add(image);
    let material = world
        .resource_mut::<Assets<ActorGpuMaterial>>()
        .add(ActorGpuMaterial { atlas: image });
    Atlas { material, rects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor_models::ScreenVertex;

    fn vertex(x: f32, y: f32, u: f32, v: f32) -> ScreenVertex {
        ScreenVertex {
            xy: [x, y],
            depth: 1.0,
            uv: [u, v],
            color: [1.0; 4],
        }
    }

    #[test]
    fn shadow_quad_is_one_mesh_with_a_shared_plate_material() {
        let mut app = App::new();
        app.add_plugins(AssetPlugin::default());
        app.init_asset::<Mesh>();
        app.init_asset::<Image>();
        app.init_asset::<ActorShadowMaterial>();
        app.init_resource::<ActorGpuCache>();
        let plate = (2usize, 2usize, vec![255u8; 16]);
        let quad = |strength| ShadowQuad {
            corners: [
                vertex(0.0, 0.0, 0.0, 0.0),
                vertex(4.0, 0.0, 1.0, 0.0),
                vertex(4.0, 4.0, 1.0, 1.0),
                vertex(0.0, 4.0, 0.0, 1.0),
            ],
            strength,
        };
        let world = app.world_mut();
        let (mesh, first) = build_shadow(world, &plate, &quad(0.5));
        let (_, second) = build_shadow(world, &plate, &quad(0.25));
        assert_eq!(first, second, "one plate material for every actor");
        let mesh = world.resource::<Assets<Mesh>>().get(&mesh).unwrap();
        assert_eq!(mesh.count_vertices(), 4);
        assert_eq!(mesh.indices().unwrap().len(), 6);
        let colors = mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap();
        let bevy::mesh::VertexAttributeValues::Float32x4(colors) = colors else {
            panic!("color attribute is not Float32x4");
        };
        assert_eq!(
            colors[0],
            [0.5, 0.0, 0.0, crate::field_light::SHADOW_VERTEX_ALPHA]
        );
        // The plate's top row is v = 1 (rows run top-down).
        let bevy::mesh::VertexAttributeValues::Float32x2(uvs) =
            mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap()
        else {
            panic!("uv attribute is not Float32x2");
        };
        assert_eq!(uvs[0], [0.0, 1.0]);
        assert_eq!(uvs[2], [1.0, 0.0]);
    }
}
