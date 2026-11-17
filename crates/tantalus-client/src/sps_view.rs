//! Field SPS effects on screen: small sprite strips (flames, sparkles, smoke) the field scripts
//! place in the world (`SPSEffect._GenerateSPSMesh`, field mode).
//!
//! Each loaded effect is a mesh of quads: the effect's world position goes through the field
//! camera (`PSX.CalculateGTE_RTPT_POS`), its scale is divided by the view depth, and each
//! sprite is offset from that center by its own `sbyte << 2` units. The blend modes are
//! `PSX/FieldSPS_Abr_0`..`_3` and `_None` (`Shaders/PSX/FieldSPS_*.txt`).

use crate::actor_models::ActorModels;
use crate::projection::{layer_z, project_field_point_depth};
use crate::smooth::{ACTOR_MOVE_MAX, Pair, lerp, lerp_angle, lerp3, shortest_delta};
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
    let (sin, cos) = angle.sin_cos();
    build_sprites_with(
        file,
        frame,
        |cx, cy| {
            let (sx, sy) = (cx * scale, -cy * scale);
            Some([
                center[0] + sx * cos - sy * sin,
                center[1] + sx * sin + sy * cos,
            ])
        },
        colors,
    )
}

/// `Quaternion.Euler(x, y, z)` as a matrix (Unity applies z, then x, then y), degrees in.
fn euler_matrix(degrees: [f32; 3]) -> [[f32; 3]; 3] {
    let [x, y, z] = degrees.map(f32::to_radians);
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let mul = |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| {
        let mut out = [[0.0f32; 3]; 3];
        for (row, out_row) in out.iter_mut().enumerate() {
            for (column, value) in out_row.iter_mut().enumerate() {
                *value = (0..3).map(|k| a[row][k] * b[k][column]).sum();
            }
        }
        out
    };
    mul(ry, mul(rx, rz))
}

/// Where the Crystal World teleport effect (field 2929) puts a sprite corner
/// (`SPSEffect._GenerateSPSMesh`, `useScreenPositionHack`): the corner `(cx, cy)` goes through
/// `TRS(pos * 0.9925, Euler(-rot.x / 2, -rot.y / 2, rot.z / 2), (scale, -scale, 1))` and then
/// straight through the field camera, with no per-sprite depth division of the size.
pub fn screen_hack_corner(
    camera: &LegacyFieldCamera,
    pos: [f32; 3],
    scale: f32,
    rot: [f32; 3],
    corner: [f32; 2],
) -> Option<[f32; 2]> {
    let matrix = euler_matrix([-rot[0] / 2.0, -rot[1] / 2.0, rot[2] / 2.0]);
    let local = [corner[0] * scale, -corner[1] * scale, 0.0];
    let point: [f32; 3] = std::array::from_fn(|row| {
        pos[row] * 0.9925 + (0..3).map(|k| matrix[row][k] * local[k]).sum::<f32>()
    });
    project_field_point_depth(camera, point).map(|(screen, _)| screen)
}

/// [`build_sprites`] with the corner placement left to `place(x, y)`, which gets the corner's
/// sprite-space offset and returns its screen position (`None` drops the effect).
pub fn build_sprites_with(
    file: &SpsFile,
    frame: usize,
    place: impl Fn(f32, f32) -> Option<[f32; 2]>,
    colors: impl Fn([u8; 3]) -> [f32; 3],
) -> Option<Mesh> {
    let prims = file.frames.get(frame)?;
    if prims.is_empty() {
        return None;
    }
    let (texel_w, texel_h) = file.texel_size();
    // `uvShrink`: field effects sample half a texel in from each edge.
    let (w, h) = (texel_w as f32, texel_h as f32);
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
            let [px, py] = place(cx, cy)?;
            positions.push([px, py, z]);
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
    // `CommonSPSSystem` and `SPSEffect` set the page's filter with
    // `ModelFactory.GetFilterMode(SFXSmoothTexture, 1)`: bilinear unless the player asks for
    // point sampling (the background atlas, `BGSCENE_DEF`, is the one set to point).
    image.sampler = ImageSampler::linear();
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
        // `useScreenPositionHack`: the Crystal World teleport effect (field 2929, `gMode` 1).
        let hack = source.field.0 == 2929;
        let depth_at = if hack {
            at.map(|value| value * 0.9925)
        } else {
            at
        };
        let Some(([x, y], depth)) = project_field_point_depth(camera, depth_at) else {
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
        let mesh = if hack {
            let size = slot.scale as f32 / 4096.0;
            build_sprites_with(
                file,
                frame_index,
                |cx, cy| screen_hack_corner(camera, at, size, slot.rot, [cx, cy]),
                |color| sprite_color(color, fade, field_id, id),
            )
        } else {
            build_sprites(
                file,
                frame_index,
                [x, y],
                scale,
                (-slot.rot[2]).to_radians(),
                |color| sprite_color(color, fade, field_id, id),
            )
        };
        let Some(mesh) = mesh else {
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

/// The slots `t` of the way from the previous logic tick to this one. A slot that holds the same
/// effect on both ticks moves its position, offset, size and turn smoothly; a new or changed
/// effect, a move past [`ACTOR_MOVE_MAX`] (the actors' cut) or a turn past 45 degrees is drawn
/// as it is. The frame counter, fade and blend mode are never blended.
/// (Memoria itself leaves SPS unsmoothed: `SmoothFrameUpdater_Field`'s TODO.)
pub fn blend_slots(prev: &[SpsSlot], cur: &[SpsSlot], t: f32) -> Vec<SpsSlot> {
    cur.iter()
        .enumerate()
        .map(|(index, now)| {
            let Some(before) = prev.get(index) else {
                return now.clone();
            };
            let same = before.loaded
                && now.loaded
                && before.id == now.id
                && before.origin == now.origin
                && before.char_uid == now.char_uid
                && before.bone == now.bone;
            let step = |a: [f32; 3], b: [f32; 3]| {
                ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt()
            };
            let turn = (0..3)
                .any(|axis| shortest_delta(before.rot[axis], now.rot[axis], 360.0).abs() > 45.0);
            if !same || step(before.pos, now.pos) > ACTOR_MOVE_MAX || turn {
                return now.clone();
            }
            let mut slot = now.clone();
            slot.pos = lerp3(before.pos, now.pos, t);
            slot.pos_offset = lerp3(before.pos_offset, now.pos_offset, t);
            slot.scale = lerp(before.scale as f32, now.scale as f32, t).round() as i32;
            slot.rot =
                std::array::from_fn(|axis| lerp_angle(before.rot[axis], now.rot[axis], t, 360.0));
            slot
        })
        .collect()
}

impl Pair<Vec<SpsSlot>> {
    /// The effects drawn `t` into the tick.
    pub fn slots(&self, t: f32) -> Vec<SpsSlot> {
        let Some(cur) = self.current() else {
            return Vec::new();
        };
        match self.previous() {
            Some(prev) => blend_slots(prev, cur, t),
            None => cur.clone(),
        }
    }
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
    slots: &[SpsSlot],
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
            slots,
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
    use crate::projection::projection_offset;
    use tantalus_assets::pc::field_sps::SpsPrim;
    use tantalus_contracts::Point3;

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
    fn screen_hack_projects_the_quad_corners_through_the_camera() {
        // Identity rotation, a camera 1000 units from the origin, view distance 500.
        let camera = LegacyFieldCamera {
            projection: 500,
            rotation: [4096, 0, 0, 0, 4096, 0, 0, 0, 4096],
            translation: Point3 {
                x: 0,
                y: 0,
                z: 1000,
            },
            center_offset: [0, 0],
            size: [320, 224],
            viewport_x: [0, 0],
            viewport_y: [0, 0],
            depth_offset: 0,
        };
        let at = [0.0, 0.0, 0.0];
        let center = screen_hack_corner(&camera, at, 1.0, [0.0; 3], [0.0, 0.0]).unwrap();
        let off = projection_offset(&camera);
        assert!((center[0] - off[0]).abs() < 1e-3 && (center[1] - off[1]).abs() < 1e-3);
        // A corner 100 units right and 100 down in sprite space: x grows, y shrinks, with the
        // plain 500 / 1000 perspective division (no per-sprite scale change).
        let corner = screen_hack_corner(&camera, at, 1.0, [0.0; 3], [100.0, 100.0]).unwrap();
        assert!((corner[0] - off[0] - 50.0).abs() < 1e-3);
        assert!((corner[1] - off[1] + 50.0).abs() < 1e-3);
        // The rotation is half the script's: 180 degrees about z turns the corner to 90.
        let turned = screen_hack_corner(&camera, at, 1.0, [0.0, 0.0, 180.0], [100.0, 0.0]).unwrap();
        assert!(turned[1] - off[1] > 49.0 && (turned[0] - off[0]).abs() < 1e-2);
    }

    #[test]
    fn effects_blend_between_ticks_and_cut_on_change() {
        let slot = |id: i32, x: f32, scale: i32| SpsSlot {
            loaded: true,
            id,
            pos: [x, 0.0, 0.0],
            scale,
            ..SpsSlot::default()
        };
        let blended = blend_slots(&[slot(5, 0.0, 4096)], &[slot(5, 100.0, 8192)], 0.5);
        assert_eq!(blended[0].pos[0], 50.0);
        assert_eq!(blended[0].scale, 6144);
        // Another effect in the slot, or a jump past the cut, is drawn at once.
        assert_eq!(
            blend_slots(&[slot(5, 0.0, 4096)], &[slot(6, 100.0, 4096)], 0.5)[0].pos[0],
            100.0
        );
        assert_eq!(
            blend_slots(&[slot(5, 0.0, 4096)], &[slot(5, 500.0, 4096)], 0.5)[0].pos[0],
            500.0
        );
        // A turn the short way round 0 degrees is blended across it.
        let mut a = slot(5, 0.0, 4096);
        let mut b = slot(5, 0.0, 4096);
        a.rot[2] = 350.0;
        b.rot[2] = 10.0;
        let turned = blend_slots(&[a], &[b], 0.5)[0].rot[2];
        assert!(turned.abs() < 1e-3 || (turned - 360.0).abs() < 1e-3);
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
