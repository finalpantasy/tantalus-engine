//! Actor tracks recorded by `tantalus-trace chain --actor-track`: one field
//! visit per file, a header naming the background, then the field world's
//! actor positions every two ticks. The viewer draws each actor as a marker
//! at its projected foot position over the field background.

use crate::actor_models::{ActorModels, ActorPose};
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
    for (id, position) in &frame.actors {
        if !placed.contains(id) {
            continue;
        }
        if frame
            .poses
            .get(id)
            .is_some_and(|pose| pose.model.is_none() || !pose.visible)
        {
            continue;
        }
        let raster = models
            .zip(frame.poses.get(id))
            .and_then(|(models, pose)| models.render(camera, *position, pose));
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
