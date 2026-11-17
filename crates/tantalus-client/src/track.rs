//! Actor tracks recorded by `tantalus-trace chain --actor-track`: one field
//! visit per file, a header naming the background, then the field world's
//! actor positions every two ticks. The viewer draws each actor as a marker
//! at its projected foot position over the field background.

use crate::actor_models::{ActorModels, ActorPose, Raster};
use crate::projection::project_field_point;
use crate::scene_adapter::ClientError;
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tantalus_contracts::LegacyFieldCamera;

/// Field ticks each track line covers.
pub const TICKS_PER_FRAME: u64 = 2;
/// FF9 field logic rate: 30 ticks a second, so 15 track frames.
pub const FRAMES_PER_SECOND: f64 = 15.0;

#[derive(Debug, Clone, PartialEq)]
pub struct TrackFrame {
    pub tick: u64,
    pub player: Option<u64>,
    pub actors: Vec<(u64, [f32; 3])>,
    /// Model, animation and facing per actor; empty for recorded tracks.
    pub poses: BTreeMap<u64, ActorPose>,
    /// Actors carried by another's bone: `(carried, carrier, bone, offset)`.
    pub attachments: Vec<(u64, u64, i32, [i32; 3])>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActorTrack {
    pub map: i32,
    pub field: String,
    pub frames: Vec<TrackFrame>,
}

impl ActorTrack {
    pub fn load(path: &Path) -> Result<Self, ClientError> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| ClientError::new(format!("{}: {error}", path.display())))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, ClientError> {
        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        let header: serde_json::Value = lines
            .next()
            .ok_or_else(|| ClientError::new("track is empty"))
            .and_then(parse_line)?;
        let field = header["field"]
            .as_str()
            .ok_or_else(|| ClientError::new("track header has no background name"))?
            .to_owned();
        let map = header["map"].as_i64().unwrap_or(-1) as i32;
        let mut frames = Vec::new();
        for line in lines {
            let value = parse_line(line)?;
            let actors = value["actors"]
                .as_array()
                .map(|actors| {
                    actors
                        .iter()
                        .filter_map(|actor| {
                            let row = actor.as_array()?;
                            let number = |index: usize| row.get(index)?.as_f64();
                            Some((
                                row.first()?.as_u64()?,
                                [number(1)? as f32, number(2)? as f32, number(3)? as f32],
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            frames.push(TrackFrame {
                tick: value["tick"].as_u64().unwrap_or(0),
                player: value["player"].as_u64(),
                actors,
                poses: BTreeMap::new(),
                attachments: Vec::new(),
            });
        }
        Ok(Self { map, field, frames })
    }

    /// Actors that leave the origin at some point; objects without a model
    /// stay at (0, 0, 0) for the whole visit and are not drawn.
    pub fn placed_actors(&self) -> BTreeSet<u64> {
        self.frames
            .iter()
            .flat_map(|frame| frame.actors.iter())
            .filter(|(_, position)| position.iter().any(|value| *value != 0.0))
            .map(|(id, _)| *id)
            .collect()
    }
}

fn parse_line(line: &str) -> Result<serde_json::Value, ClientError> {
    serde_json::from_str(line).map_err(|error| ClientError::new(format!("bad track line: {error}")))
}

/// One drawn actor; markers are replaced each frame.
#[derive(Component)]
pub struct TrackMarker;

/// What [`spawn_frame_markers`] drew.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Drawn {
    pub actors: usize,
    /// Of those, the ones drawn as their model rather than a box.
    pub models: usize,
}

/// Draws `frame`'s placed actors. An actor whose model `models` has
/// loaded is drawn as that model; the rest are 12x24 standing boxes whose
/// bottom edge is the foot point, the controlled character in amber, the
/// others in cyan. An actor whose pose has no model or no "show model"
/// flag is not drawn, as in the game.
pub fn spawn_frame_markers(
    world: &mut World,
    camera: &LegacyFieldCamera,
    frame: &TrackFrame,
    placed: &BTreeSet<u64>,
    models: Option<&ActorModels>,
) -> Drawn {
    let mut old = world.query_filtered::<Entity, With<TrackMarker>>();
    let old: Vec<Entity> = old.iter(world).collect();
    for entity in old {
        world.entity_mut(entity).despawn();
    }
    let mut drawn = Drawn::default();
    // The software rasterizer is the frame's cost: each actor's body and
    // shadow rasters are independent, so they run on their own threads.
    let mut rasters: BTreeMap<u64, (Option<Raster>, Option<Raster>)> = BTreeMap::new();
    let mut quads: BTreeMap<u64, crate::actor_models::ShadowQuad> = BTreeMap::new();
    // With the GPU plugin the body is drawn as a mesh from its triangles.
    let gpu = world.contains_resource::<crate::actor_gpu::ActorGpuCache>()
        && std::env::var_os("TANTALUS_ACTOR_CPU").is_none();
    // TANTALUS_SHADOW_CPU keeps the shadows on the CPU raster (comparison).
    let cpu_shadow = std::env::var_os("TANTALUS_SHADOW_CPU").is_some();
    // TANTALUS_SHADOW_OFF draws none (GPU path only; comparison).
    let no_shadow = std::env::var_os("TANTALUS_SHADOW_OFF").is_some();
    let mut triangles: BTreeMap<u64, Vec<crate::actor_models::Triangle<'_>>> = BTreeMap::new();
    // Carried actors are drawn on their carrier's bone, not where they stand.
    let carried_ids: BTreeSet<u64> = frame
        .attachments
        .iter()
        .filter(|(_, carrier, _, _)| {
            frame
                .poses
                .get(carrier)
                .is_some_and(|pose| pose.model.is_some())
        })
        .map(|(carried, ..)| *carried)
        .collect();
    if let Some(models) = models {
        let jobs: Vec<(
            u64,
            [f32; 3],
            &ActorPose,
            Vec<crate::actor_models::CarriedPose>,
        )> = frame
            .actors
            .iter()
            .filter(|(id, _)| placed.contains(id) && !carried_ids.contains(id))
            .filter_map(|(id, position)| {
                let pose = frame.poses.get(id)?;
                let carried = frame
                    .attachments
                    .iter()
                    .filter(|(_, carrier, _, _)| carrier == id)
                    .filter_map(|(carried, _, bone, offset)| {
                        Some(crate::actor_models::CarriedPose {
                            pose: *frame.poses.get(carried)?,
                            bone: *bone,
                            offset: *offset,
                        })
                    })
                    .collect();
                (pose.model.is_some() && pose.visible).then_some((*id, *position, pose, carried))
            })
            .collect();
        let raster_start = std::time::Instant::now();
        std::thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .iter()
                .map(|(id, position, pose, carried)| {
                    let (id, position, pose) = (*id, *position, *pose);
                    scope.spawn(move || {
                        if gpu {
                            let body = models.carrying_triangles(camera, position, pose, carried);
                            let casts =
                                !no_shadow && body.as_ref().is_some_and(|body| !body.is_empty());
                            if cpu_shadow {
                                let shadow = casts
                                    .then(|| models.render_shadow(camera, position, pose))
                                    .flatten();
                                return (id, None, body, shadow, None);
                            }
                            let quad = casts
                                .then(|| models.shadow_quad(camera, position, pose))
                                .flatten();
                            return (id, None, body, None, quad);
                        }
                        let body = models.render_carrying(camera, position, pose, carried);
                        let shadow = body
                            .as_ref()
                            .and_then(|_| models.render_shadow(camera, position, pose));
                        (id, body, None, shadow, None)
                    })
                })
                .collect();
            for handle in handles {
                if let Ok((id, body, mesh, shadow, quad)) = handle.join() {
                    rasters.insert(id, (body, shadow));
                    if let Some(quad) = quad {
                        quads.insert(id, quad);
                    }
                    if let Some(mesh) = mesh {
                        triangles.insert(id, mesh);
                    }
                }
            }
        });
        crate::live::bench::record("m_raster", raster_start.elapsed().as_secs_f64() * 1000.0);
        crate::live::bench::record("m_jobs", jobs.len() as f64);
    }
    for (id, position) in &frame.actors {
        if !placed.contains(id) || carried_ids.contains(id) {
            continue;
        }
        if frame
            .poses
            .get(id)
            .is_some_and(|pose| pose.model.is_none() || !pose.visible)
        {
            continue;
        }
        let (raster, shadow) = rasters.remove(id).unwrap_or_default();
        if let Some(body) = triangles.remove(id)
            && let Some(actor) = crate::actor_gpu::build_actor(world, &body)
        {
            let Some(depth) = crate::projection::actor_depth(camera, *position) else {
                continue;
            };
            if !(100.0..=3996.0).contains(&depth) {
                continue;
            }
            let z = crate::projection::layer_z(depth) + 0.01 / (1.0 + actor.depth.max(0.0));
            if let Some(shadow) = shadow {
                spawn_shadow(world, shadow, z - 0.005);
            }
            if let Some(quad) = quads.remove(id)
                && let Some(plate) = models.and_then(|models| models.shadow_plate())
            {
                let (mesh, material) = crate::actor_gpu::build_shadow(world, plate, &quad);
                world.spawn((
                    Mesh2d(mesh),
                    MeshMaterial2d(material),
                    Transform::from_xyz(0.0, 0.0, z - 0.005),
                    TrackMarker,
                ));
            }
            world.spawn((
                Mesh2d(actor.mesh),
                MeshMaterial2d(actor.material),
                Transform::from_xyz(0.0, 0.0, z),
                TrackMarker,
            ));
            drawn.actors += 1;
            drawn.models += 1;
            continue;
        }
        if let Some(raster) = raster {
            let ([x, y], size) = raster.placement();
            let image = Image::new(
                bevy::render::render_resource::Extent3d {
                    width: raster.width,
                    height: raster.height,
                    depth_or_array_layers: 1,
                },
                bevy::render::render_resource::TextureDimension::D2,
                raster.rgba,
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::default(),
            );
            let handle = world.resource_mut::<Assets<Image>>().add(image);
            // One depth per character, from its root position, on the
            // background's depth scale; ties between characters go to
            // the nearer raster. `FieldMapActor` turns the character off
            // outside depths 100..=3996.
            let Some(depth) = crate::projection::actor_depth(camera, *position) else {
                continue;
            };
            if !(100.0..=3996.0).contains(&depth) {
                continue;
            }
            let z = crate::projection::layer_z(depth) + 0.01 / (1.0 + raster.depth.max(0.0));
            if let Some(shadow) = shadow {
                // Drawn just before its actor, on the same layer.
                spawn_shadow(world, shadow, z - 0.005);
            }
            world.spawn((
                Sprite {
                    image: handle,
                    custom_size: Some(Vec2::new(size[0], size[1])),
                    ..default()
                },
                Transform::from_xyz(x, y, z),
                TrackMarker,
            ));
            drawn.actors += 1;
            drawn.models += 1;
            continue;
        }
        let Some([x, y]) = project_field_point(camera, *position) else {
            continue;
        };
        let color = if frame.player == Some(*id) {
            Color::srgb(1.0, 0.75, 0.1)
        } else {
            Color::srgb(0.2, 0.85, 1.0)
        };
        world.spawn((
            Sprite::from_color(color.with_alpha(0.85), Vec2::new(12.0, 24.0)),
            Transform::from_xyz(x, y + 12.0, 460.0),
            TrackMarker,
        ));
        drawn.actors += 1;
    }
    drawn
}

/// One shadow raster as a reverse-subtract quad (`PSX/FieldMapActorShadow`
/// blends `One One RevSub`). Skipped when the blend material is not set up.
fn spawn_shadow(world: &mut World, raster: crate::actor_models::Raster, z: f32) {
    use crate::psx_blend::PsxBlendMaterial;
    let ([x, y], size) = raster.placement();
    let pixels = [raster.width as f32, raster.height as f32];
    let image = Image::new(
        bevy::render::render_resource::Extent3d {
            width: raster.width,
            height: raster.height,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        raster.rgba,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );
    if world.get_resource::<Assets<PsxBlendMaterial>>().is_none() {
        return;
    }
    let image = world.resource_mut::<Assets<Image>>().add(image);
    let Some(material) = PsxBlendMaterial::new(image, [0.0, 0.0, pixels[0], pixels[1]], pixels, 2)
    else {
        return;
    };
    let material = world
        .resource_mut::<Assets<PsxBlendMaterial>>()
        .add(material);
    let mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Rectangle::new(size[0], size[1]));
    world.spawn((
        Mesh2d(mesh),
        MeshMaterial2d(material),
        Transform::from_xyz(x, y, z),
        TrackMarker,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_frames_and_placed_actors() {
        let text = concat!(
            "{\"field\":\"FBG_X\",\"map\":262,\"scenario_counter\":2375}\n",
            "{\"actors\":[[1,0.0,0.0,0.0,0],[2,5.0,0.0,9.0,64]],\"control\":false,\"player\":2,\"tick\":0}\n",
            "{\"actors\":[[1,0.0,0.0,0.0,0],[2,6.0,0.0,9.0,64]],\"control\":true,\"player\":2,\"tick\":2}\n",
        );
        let track = ActorTrack::parse(text).unwrap();
        assert_eq!(track.field, "FBG_X");
        assert_eq!(track.map, 262);
        assert_eq!(track.frames.len(), 2);
        assert_eq!(track.frames[1].actors[1], (2, [6.0, 0.0, 9.0]));
        assert_eq!(track.placed_actors(), BTreeSet::from([2]));
    }
}
