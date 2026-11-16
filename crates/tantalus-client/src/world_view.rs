//! The world map view: the blocks around the player drawn through the world
//! camera into a CPU frame ([`crate::battle_raster`]), and the live-window
//! sprite that shows it.
//!
//! Block geometry is local to the block; block `(x, y)` sits at
//! `block_origin(x, y)` (`WMWorld.OnInitialize`). The camera is
//! `w_cameraSetEyeAim`'s ([`tantalus_tools::world_play::WorldPlay::camera`]).

use std::collections::HashMap;
use std::sync::Arc;

use bevy::math::{Quat, Vec3};
use tantalus_assets::pc::battle_background::BattleBackground;
use tantalus_tools::world_data::mesh_in_form;
use tantalus_tools::world_play::WorldPlay;

use crate::actor_models::{ActorModels, ActorPose};
use crate::battle_raster::{
    BattleCamera, Blend, DrawState, Frame, PSX_HEIGHT, Texture, ViewVertex,
};
use crate::world_minimap::Minimap;
use crate::world_weather::Weather;

/// Blocks drawn each side of the player's block.
const RADIUS: i32 = 4;
/// Loaded blocks kept past the drawn ring before the farthest go.
const KEEP: i32 = 6;
/// How much of the water shows over what is under it (`Sea*` meshes).
const WATER_OPACITY: u8 = 190;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    Land,
    Water,
}

/// Render layer of the live world sprite.
pub const WORLD_LAYER: usize = 8;

/// `GEO_SUB_W0_001`, Zidane's world model, and its `ANH_SUB_W0_001_STAND`
/// and `_RUN` clips (`WMAnimationBank`).
const ZIDANE_MODEL: u16 = 310;
const ZIDANE_IDLE: u16 = 4719;
const ZIDANE_RUN: u16 = 4722;
/// `GEO_SUB_W0_003`, the chocobo, with `ANH_SUB_W0_003_IDLE1_CHO`, `_RUN_CHO`
/// and `_FLY_CHO` (`WMAnimationBank`).
const CHOCOBO_MODEL: u16 = 308;
const CHOCOBO_IDLE: u16 = 4707;
const CHOCOBO_RUN: u16 = 4710;
const CHOCOBO_FLY: u16 = 4991;
/// Zidane mounted: `ANH_SUB_W0_001_IDLE1_CHO`, `_RUN_CHO` and `_FLY_CHO`,
/// authored on the chocobo's own origin, so he is drawn with the chocobo's
/// transform.
const RIDER_IDLE: u16 = 4724;
const RIDER_RUN: u16 = 4727;
const RIDER_FLY: u16 = 1143;
/// `GEO_SUB_W0_008`, the Blue Narciss, and its `ANH_SUB_W0_008_IDLE1_BN`
/// clip; `WMActor.SetScale(106, 106, 106)`.
const NARCISS_MODEL: u16 = 321;
const NARCISS_IDLE: u16 = 5144;
const NARCISS_SCALE: f32 = 106.0 / 64.0 / 256.0;
/// `WMActor.SetScale(67, 67, 67)`: 67 / 64 of 1/256.
const MODEL_SCALE: f32 = 67.0 / 64.0 / 256.0;
/// Hue turn in degrees and brightness that stand in for the other chocobo
/// colors (control entries 1 to 6: yellow, light blue, red, deep blue, gold
/// on the ground and in the air). The reference swaps in textures from
/// Memoria's embedded assets, which an install does not carry.
const CHOCOBO_TINTS: [(f32, f32); 6] = [
    (0.0, 1.0),
    (140.0, 1.0),
    (-50.0, 0.95),
    (185.0, 0.9),
    (-8.0, 1.15),
    (-8.0, 1.15),
];

pub struct WorldRenderer {
    models: ActorModels,
    minimap: Minimap,
    blocks: HashMap<(usize, usize), Option<Arc<BattleBackground>>>,
    /// Blocks that failed, for reports.
    pub problems: Vec<String>,
}

impl WorldRenderer {
    /// `streaming` is the `StreamingAssets` folder.
    pub fn new(streaming: &std::path::Path) -> Self {
        let mut models = ActorModels::new(streaming);
        let poses = [
            (ZIDANE_MODEL, ZIDANE_IDLE),
            (ZIDANE_MODEL, ZIDANE_RUN),
            (CHOCOBO_MODEL, CHOCOBO_IDLE),
            (CHOCOBO_MODEL, CHOCOBO_RUN),
            (CHOCOBO_MODEL, CHOCOBO_FLY),
            (ZIDANE_MODEL, RIDER_IDLE),
            (ZIDANE_MODEL, RIDER_RUN),
            (ZIDANE_MODEL, RIDER_FLY),
            (NARCISS_MODEL, NARCISS_IDLE),
        ]
        .map(|(model, animation)| ActorPose {
            model: Some(model),
            animation: Some(animation),
            ..ActorPose::default()
        });
        let problems = models.prepare(&poses);
        Self {
            models,
            minimap: Minimap::default(),
            blocks: HashMap::new(),
            problems,
        }
    }
}

fn camera_for(play: &WorldPlay) -> BattleCamera {
    let view = play.camera();
    let eye = Vec3::from_array(view.eye);
    let forward = (Vec3::from_array(view.aim) - eye).normalize_or_zero();
    let yaw = forward.x.atan2(forward.z).to_degrees();
    let pitch = (-forward.y).asin().to_degrees();
    // The PSX projection spans 220 rows; its half is the vertical half angle.
    let near = (PSX_HEIGHT / 2.0) / (view.fov.to_radians() / 2.0).tan();
    BattleCamera::from_transform(eye, [pitch, yaw, 0.0], near)
}

impl WorldRenderer {
    fn block(&mut self, play: &WorldPlay, x: usize, y: usize) -> Option<Arc<BattleBackground>> {
        if let Some(entry) = self.blocks.get(&(x, y)) {
            return entry.clone();
        }
        let loaded = if play.assets.has_block(x, y) {
            match play.assets.render_block(x, y) {
                Ok(block) => Some(Arc::new(block)),
                Err(error) => {
                    self.problems.push(format!("block [{x}][{y}]: {error}"));
                    None
                }
            }
        } else {
            None
        };
        self.blocks.insert((x, y), loaded.clone());
        loaded
    }

    /// Draws the world around `play`.
    pub fn render(&mut self, play: &WorldPlay, width: u32, height: u32) -> Frame {
        let camera = camera_for(play);
        let weather = Weather::at(play.walker.pos.x, play.walker.pos.z);
        let mut frame = Frame::new(
            width,
            height,
            [weather.sky[0], weather.sky[1], weather.sky[2], 255],
        );
        let pos = play.walker.pos;
        let here = (
            (pos.x / 64.0).floor() as i32,
            (pos.z.abs() / 64.0).floor() as i32,
        );
        let apart = |a: i32, b: i32, size: i32| {
            let d = (a - b).rem_euclid(size);
            d.min(size - d)
        };
        self.blocks.retain(|&(x, y), _| {
            apart(x as i32, here.0, 24) <= KEEP && apart(y as i32, here.1, 20) <= KEEP
        });
        let mut drawn: Vec<Drawn> = Vec::new();
        for dy in -RADIUS..=RADIUS {
            for dx in -RADIUS..=RADIUS {
                // The world wraps: past an edge the far side's blocks follow.
                let (ux, uy) = (here.0 + dx, here.1 + dy);
                let (x, y) = (ux.rem_euclid(24), uy.rem_euclid(20));
                if let Some(block) = self.block(play, x as usize, y as usize) {
                    drawn.push(Drawn {
                        block,
                        origin: [ux as f32 * 64.0, 0.0, -(uy as f32) * 64.0],
                        key: (x as usize, y as usize),
                        form: if play.forms.contains(&(x as usize, y as usize)) {
                            2
                        } else {
                            1
                        },
                    });
                }
            }
        }
        for pass in [Pass::Land, Pass::Water] {
            for item in &drawn {
                draw_block(&mut frame, &camera, item, pass);
            }
        }
        let moving = play.walker.moved;
        let time = play.frames as f32 / 30.0;
        let at = [pos.x, pos.y, pos.z];
        let place = |scale: f32, tint: (f32, f32)| Placement {
            at,
            yaw: play.walker.rot_y,
            scale,
            tint,
        };
        let plain = (0.0, 1.0);
        let drawn_player = match play.ride {
            0 => {
                let clip = if moving { ZIDANE_RUN } else { ZIDANE_IDLE };
                draw_character(
                    &mut frame,
                    &camera,
                    &self.models,
                    (ZIDANE_MODEL, clip),
                    &place(MODEL_SCALE, plain),
                    time,
                )
            }
            7 => draw_character(
                &mut frame,
                &camera,
                &self.models,
                (NARCISS_MODEL, NARCISS_IDLE),
                &place(NARCISS_SCALE, plain),
                time,
            ),
            ride => {
                let (mount, rider) = match (ride, moving) {
                    (6, _) => (CHOCOBO_FLY, RIDER_FLY),
                    (_, true) => (CHOCOBO_RUN, RIDER_RUN),
                    (_, false) => (CHOCOBO_IDLE, RIDER_IDLE),
                };
                let tint = CHOCOBO_TINTS[usize::from(ride) - 1];
                let drew = draw_character(
                    &mut frame,
                    &camera,
                    &self.models,
                    (CHOCOBO_MODEL, mount),
                    &place(MODEL_SCALE, tint),
                    time,
                );
                draw_character(
                    &mut frame,
                    &camera,
                    &self.models,
                    (ZIDANE_MODEL, rider),
                    &place(MODEL_SCALE, plain),
                    time,
                );
                drew
            }
        };
        if !drawn_player {
            draw_marker(&mut frame, &camera, at, play.walker.rot_y);
        }
        let view = play.camera();
        let (dx, dz) = (view.aim[0] - view.eye[0], view.aim[2] - view.eye[2]);
        let flat = dx.hypot(dz).max(1e-3);
        let far = [
            view.eye[0] + dx / flat * 5000.0,
            view.eye[1],
            view.eye[2] + dz / flat * 5000.0,
        ];
        let horizon = frame
            .project(&camera, camera.to_view(far))
            .map_or(height as f32 * 0.4, |p| p[1]);
        weather.apply(&mut frame, horizon);
        self.minimap.draw(&mut frame, play);
        frame
    }
}

/// A block to draw where it stands this frame.
struct Drawn {
    block: Arc<BattleBackground>,
    origin: [f32; 3],
    key: (usize, usize),
    form: u8,
}

fn draw_block(frame: &mut Frame, camera: &BattleCamera, item: &Drawn, pass: Pass) {
    let (block, origin) = (&item.block, item.origin);
    for mesh in &block.meshes {
        if !mesh_in_form(&mesh.name, item.form, item.key) {
            continue;
        }
        let view: Vec<Vec3> = mesh
            .positions
            .iter()
            .map(|p| camera.to_view([p[0] + origin[0], p[1] + origin[1], p[2] + origin[2]]))
            .collect();
        for submesh in &mesh.submeshes {
            if submesh.topology != 0 {
                continue;
            }
            // The battle rule that reads blending off the material name
            // does not apply to the world; only the sea blends.
            let water = mesh.name.starts_with("Sea");
            let (blend, depth_write) = if water {
                (Blend::Alpha(WATER_OPACITY), false)
            } else {
                (Blend::Opaque, true)
            };
            let this_pass = if water { Pass::Water } else { Pass::Land };
            if this_pass != pass {
                continue;
            }
            let state = DrawState {
                blend,
                depth_test: true,
                depth_write,
                cull_back: true,
            };
            let texture = submesh
                .texture
                .and_then(|index| block.textures.get(index))
                .map(|texture| Texture {
                    width: texture.width,
                    height: texture.height,
                    rgba: &texture.rgba,
                    clamp: false,
                });
            let first = submesh.first_index as usize;
            let end = first + submesh.index_count as usize;
            let Some(indices) = mesh.indices.get(first..end) else {
                continue;
            };
            for corner in indices.chunks_exact(3) {
                let vertex = |at: u32| {
                    Some(ViewVertex {
                        position: *view.get(at as usize)?,
                        uv: mesh.uvs.get(at as usize).copied().unwrap_or_default(),
                    })
                };
                if let (Some(a), Some(b), Some(c)) =
                    (vertex(corner[0]), vertex(corner[1]), vertex(corner[2]))
                {
                    frame.draw(camera, [a, b, c], texture, state);
                }
            }
        }
    }
}

/// Where and how big a model stands.
struct Placement {
    at: [f32; 3],
    /// Degrees about the up axis.
    yaw: f32,
    scale: f32,
    /// Hue turn in degrees and brightness of the texture.
    tint: (f32, f32),
}

/// `rgba` with its hue turned `degrees` and its brightness scaled by `gain`.
fn tinted(rgba: &[u8], (degrees, gain): (f32, f32)) -> Vec<u8> {
    let (sin, cos) = degrees.to_radians().sin_cos();
    // The standard hue rotation about the gray axis.
    let m = [
        [
            0.213 + cos * 0.787 - sin * 0.213,
            0.715 - cos * 0.715 - sin * 0.715,
            0.072 - cos * 0.072 + sin * 0.928,
        ],
        [
            0.213 - cos * 0.213 + sin * 0.143,
            0.715 + cos * 0.285 + sin * 0.140,
            0.072 - cos * 0.072 - sin * 0.283,
        ],
        [
            0.213 - cos * 0.213 - sin * 0.787,
            0.715 - cos * 0.715 + sin * 0.715,
            0.072 + cos * 0.928 + sin * 0.072,
        ],
    ];
    let mut out = rgba.to_vec();
    for pixel in out.chunks_exact_mut(4) {
        let rgb = [
            f32::from(pixel[0]),
            f32::from(pixel[1]),
            f32::from(pixel[2]),
        ];
        for (channel, row) in m.iter().enumerate() {
            let value = (row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2]) * gain;
            pixel[channel] = value.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// A skinned world model, turned and scaled as `place` says. False when the
/// model did not load.
fn draw_character(
    frame: &mut Frame,
    camera: &BattleCamera,
    models: &ActorModels,
    clip: (u16, u16),
    place: &Placement,
    time: f32,
) -> bool {
    let (at, yaw) = (place.at, place.yaw);
    let Some((model, skinned)) = models.skinned(clip.0, Some(clip.1), time) else {
        return false;
    };
    // The model space is y-down: turn it upright, then about the up axis.
    let rotation =
        Quat::from_rotation_y(yaw.to_radians()) * Quat::from_rotation_z(std::f32::consts::PI);
    let origin = Vec3::from_array(at);
    let state = DrawState {
        blend: Blend::Opaque,
        depth_test: true,
        depth_write: true,
        cull_back: true,
    };
    for (part, vertices) in model.parts.iter().zip(&skinned) {
        let view: Vec<Vec3> = vertices
            .iter()
            .map(|vertex| {
                let world = origin + rotation * (Vec3::from_array(*vertex) * place.scale);
                camera.to_view(world.to_array())
            })
            .collect();
        for submesh in &part.submeshes {
            if submesh.topology != 0 {
                continue;
            }
            let source = submesh.material.as_ref().and_then(|material| {
                material.textures.iter().find_map(|texture| {
                    let rgba = texture.rgba.as_deref()?;
                    let width = usize::try_from(texture.width).ok()?;
                    let height = usize::try_from(texture.height).ok()?;
                    (width > 0 && height > 0 && rgba.len() >= width * height * 4)
                        .then_some((width, height, rgba))
                })
            });
            let recolored = source
                .filter(|_| place.tint != (0.0, 1.0))
                .map(|(_, _, rgba)| tinted(rgba, place.tint));
            let texture = source.map(|(width, height, rgba)| Texture {
                width,
                height,
                rgba: recolored.as_deref().unwrap_or(rgba),
                clamp: false,
            });
            let first = submesh.first_index as usize;
            let end = first + submesh.index_count as usize;
            let Some(indices) = part.indices.get(first..end) else {
                continue;
            };
            for corner in indices.chunks_exact(3) {
                let vertex = |index: u32| {
                    Some(ViewVertex {
                        position: *view.get(index as usize)?,
                        uv: part.uvs.get(index as usize).copied().unwrap_or_default(),
                    })
                };
                if let (Some(a), Some(b), Some(c)) =
                    (vertex(corner[0]), vertex(corner[1]), vertex(corner[2]))
                {
                    frame.draw(camera, [a, b, c], texture, state);
                }
            }
        }
    }
    true
}

/// A small pyramid standing on the actor, turned to its yaw.
fn draw_marker(frame: &mut Frame, camera: &BattleCamera, at: [f32; 3], yaw_degrees: f32) {
    const TEXEL: [u8; 4] = [255, 210, 40, 255];
    const SIDE: [u8; 4] = [200, 60, 40, 255];
    let yaw = yaw_degrees.to_radians();
    let (sin, cos) = (yaw.sin(), yaw.cos());
    let corner = |x: f32, z: f32| [at[0] + x * cos + z * sin, at[1], at[2] - x * sin + z * cos];
    let top = [at[0], at[1] + 1.6, at[2]];
    let base = [
        corner(-0.5, -0.5),
        corner(0.5, -0.5),
        corner(0.5, 0.5),
        corner(-0.5, 0.5),
    ];
    let state = DrawState {
        blend: Blend::Opaque,
        depth_test: true,
        depth_write: true,
        cull_back: false,
    };
    let view = |p: [f32; 3]| ViewVertex {
        position: camera.to_view(p),
        uv: [0.5, 0.5],
    };
    for index in 0..4 {
        let (a, b) = (base[index], base[(index + 1) % 4]);
        let texel = if index == 0 { TEXEL } else { SIDE };
        let rgba = texel;
        let texture = Texture {
            width: 1,
            height: 1,
            rgba: &rgba,
            clamp: true,
        };
        frame.draw(camera, [view(a), view(b), view(top)], Some(texture), state);
    }
}

/// The live-window sprite showing the world frame.
pub struct LiveWorld {
    pub renderer: WorldRenderer,
    image: bevy::asset::Handle<bevy::image::Image>,
    camera: bevy::ecs::entity::Entity,
    sprite: bevy::ecs::entity::Entity,
}

/// Size of the CPU world frame in the window.
pub const LIVE_SIZE: (u32, u32) = (640, 480);

impl LiveWorld {
    pub fn start(
        world: &mut bevy::ecs::world::World,
        play: &WorldPlay,
        streaming: &std::path::Path,
    ) -> Self {
        use bevy::camera::visibility::RenderLayers;
        use bevy::camera::{RenderTarget, ScalingMode};
        use bevy::prelude::*;
        let mut renderer = WorldRenderer::new(streaming);
        let frame = renderer.render(play, LIVE_SIZE.0, LIVE_SIZE.1);
        let image = world
            .resource_mut::<Assets<Image>>()
            .add(crate::battle_view::frame_image(&frame));
        let mut targets = world.query_filtered::<&RenderTarget, With<Camera2d>>();
        let target = targets.iter(world).next().cloned();
        let (width, height) = crate::window::FIELD_FRAME;
        let camera = world
            .spawn((
                Camera2d,
                Camera {
                    order: 9,
                    clear_color: ClearColorConfig::Custom(Color::BLACK),
                    ..default()
                },
                Projection::Orthographic(OrthographicProjection {
                    scaling_mode: ScalingMode::AutoMin {
                        min_width: width,
                        min_height: height,
                    },
                    ..OrthographicProjection::default_2d()
                }),
                RenderLayers::layer(WORLD_LAYER),
            ))
            .id();
        if let Some(target) = target {
            world.entity_mut(camera).insert(target);
        }
        let sprite = world
            .spawn((
                Sprite {
                    image: image.clone(),
                    custom_size: Some(Vec2::new(width, height)),
                    ..default()
                },
                Transform::default(),
                RenderLayers::layer(WORLD_LAYER),
            ))
            .id();
        Self {
            renderer,
            image,
            camera,
            sprite,
        }
    }

    pub fn update(&mut self, world: &mut bevy::ecs::world::World, play: &WorldPlay) {
        use bevy::prelude::*;
        let frame = self.renderer.render(play, LIVE_SIZE.0, LIVE_SIZE.1);
        if let Some(mut image) = world.resource_mut::<Assets<Image>>().get_mut(&self.image) {
            image.data = Some(frame.rgba);
        }
        let at = world
            .get::<Transform>(self.camera)
            .map(|transform| transform.translation.truncate());
        if let (Some(at), Some(mut transform)) = (at, world.get_mut::<Transform>(self.sprite)) {
            transform.translation.x = at.x;
            transform.translation.y = at.y;
        }
    }

    pub fn finish(self, world: &mut bevy::ecs::world::World) {
        for entity in [self.sprite, self.camera] {
            if let Ok(entity) = world.get_entity_mut(entity) {
                entity.despawn();
            }
        }
        world
            .resource_mut::<bevy::asset::Assets<bevy::image::Image>>()
            .remove(&self.image);
    }
}
