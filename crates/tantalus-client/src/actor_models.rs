//! Field actor models for the play window: each actor's skinned FF9 model,
//! posed by its current animation, projected through the field camera and
//! rasterized on the CPU into one image per actor.
//!
//! Placement follows Memoria's field actor set-up: the model root has scale
//! `(-1, -1, 1)` (`FieldMapActor.cs`), is turned about the up axis by the
//! facing (`ConvertFixedPointAngleToDegree`: facing x 360 / 256 degrees) and
//! sits at the actor position. Models come from `p0data4.bin` through
//! `model_factory`, clips from `p0data5.bin` through `AnimationDB`
//! (`field_models::resolve_animation`). Texture sampling is affine, like
//! the PSX field renderer; the only light is the map config's per-actor
//! `_Color` and shadow (`field_light`).

use crate::bundle_store::with_store;
use crate::projection::project_field_point_depth;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use tantalus_assets::pc::field_models::{model_dirs, resolve_animation};
use tantalus_assets::pc::map_config::MapConfig;
use tantalus_assets::pc::model_assembly::pose::{
    Interpolation, NodePose, bind_clip, rest_pose, sample_clip, skin_model, transform_point,
    world_matrices,
};
use tantalus_assets::pc::model_assembly::{
    AssembleOptions, AssembledModel, ModelCatalog, assemble_model, clip_entries,
};
use tantalus_assets::pc::unity_object::animation::AnimationClip;
use tantalus_contracts::LegacyFieldCamera;

/// Raster pixels per field pixel.
pub const RASTER_SCALE: f32 = 4.0;
/// Larger rasters are skipped (an actor almost on the camera plane).
const MAX_RASTER_SIDE: u32 = 2048;
/// Unity `MeshTopology.Triangles`.
const TOPOLOGY_TRIANGLES: i32 = 0;

/// What one actor shows this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ActorPose {
    /// The field object's id, the key of its texture animation state (0 when it has none).
    pub actor: u64,
    /// 0..=255, 0 south, 64 west.
    pub facing: u8,
    pub model: Option<u16>,
    pub animation: Option<u16>,
    /// Seconds into the animation.
    pub time: f32,
    /// Object flag 1, "show model" (`SetRenderer`: no renderer is on
    /// without it).
    pub visible: bool,
    /// `meshflags`: bit i hides the `mesh<i>` child.
    pub hidden_meshes: u32,
    /// Walkmesh floor under the actor, for its floor light.
    pub floor: Option<i16>,
    /// The field scripts' color and shadow changes for this actor.
    pub script: tantalus_game::field::world::ActorShadowScript,
    /// Garnet's hair is cut (`ScenarioCounter >= 10300`): her model keeps
    /// `short_hair` instead of `long_hair` (`PosObj.shortHair`).
    pub short_hair: bool,
    /// `SetObjectScale` (`CHRSCALE`): the model's x, y (up) and z scale; `None` is unscaled.
    pub scale: Option<[f32; 3]>,
}

/// One projected vertex: raster space position, view depth and texture
/// coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenVertex {
    pub xy: [f32; 2],
    pub depth: f32,
    pub uv: [f32; 2],
    /// Vertex color, RGBA in 0..=1 (white when the mesh has none).
    pub color: [f32; 4],
}

/// An actor's shadow: the quad's corners (-1,-1), (1,-1), (1,1), (-1,1)
/// in x, z and the shadow color strength.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowQuad {
    pub corners: [ScreenVertex; 4],
    pub strength: f32,
}

/// RGBA texels, rows top-down as `Texture2D::rgba_bytes` returns them, so
/// `v = 0` (Unity's bottom row) is the last row.
#[derive(Debug, Clone, Copy)]
pub struct Texels<'a> {
    pub width: usize,
    pub height: usize,
    pub rgba: &'a [u8],
}

impl Texels<'_> {
    fn sample(&self, uv: [f32; 2]) -> [u8; 4] {
        let wrap = |value: f32, size: usize| {
            let value = value - value.floor();
            ((value * size as f32) as usize).min(size - 1)
        };
        let x = wrap(uv[0], self.width);
        let y = self.height - 1 - wrap(uv[1], self.height);
        let at = (y * self.width + x) * 4;
        self.rgba
            .get(at..at + 4)
            .map_or([255, 0, 255, 255], |texel| {
                [texel[0], texel[1], texel[2], texel[3]]
            })
    }
}

/// One projected triangle with its texture, if any.
pub type Triangle<'a> = ([ScreenVertex; 3], Option<Texels<'a>>);

/// A rasterized actor. `origin` is the field-space top-left corner
/// (x right, y up); each raster pixel is `1 / RASTER_SCALE` field pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub origin: [f32; 2],
    /// Nearest view depth drawn, for ordering actors.
    pub depth: f32,
}

impl Raster {
    /// Field-space center and size, for placing the image as a sprite.
    pub fn placement(&self) -> ([f32; 2], [f32; 2]) {
        let size = [
            self.width as f32 / RASTER_SCALE,
            self.height as f32 / RASTER_SCALE,
        ];
        (
            [
                self.origin[0] + size[0] / 2.0,
                self.origin[1] - size[1] / 2.0,
            ],
            size,
        )
    }
}

/// Draws textured triangles given in field space (x right, y up) with a
/// depth buffer, shaded as `PSX/FieldMapActor` does: the texel times the
/// vertex color (which carries `2 * _Color`), cut out where texel alpha
/// times vertex alpha is under 0.5. Triangles are double sided. `None`
/// when nothing is in front of the camera or the raster would be too
/// large.
pub fn rasterize(triangles: &[([ScreenVertex; 3], Option<Texels<'_>>)]) -> Option<Raster> {
    rasterize_with(triangles, |texel, shade| {
        if f32::from(texel[3]) / 255.0 * shade[3] < 0.5 {
            return None;
        }
        let lit = |channel: usize| {
            (f32::from(texel[channel]) * shade[channel].max(0.0))
                .round()
                .min(255.0) as u8
        };
        Some([lit(0), lit(1), lit(2), 255])
    })
}

/// [`rasterize`] with another fragment: `fragment(texel, vertex color)`
/// gives the pixel, or `None` to discard it (no depth write).
pub fn rasterize_with(
    triangles: &[([ScreenVertex; 3], Option<Texels<'_>>)],
    fragment: impl Fn([u8; 4], [f32; 4]) -> Option<[u8; 4]>,
) -> Option<Raster> {
    rasterize_rows(triangles, fragment, true)
}

/// `narrow` limits each row to the columns the triangle can cover; off, the
/// whole bounding box is scanned (the reference the tests compare to).
fn rasterize_rows(
    triangles: &[([ScreenVertex; 3], Option<Texels<'_>>)],
    fragment: impl Fn([u8; 4], [f32; 4]) -> Option<[u8; 4]>,
    narrow: bool,
) -> Option<Raster> {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for vertex in triangles.iter().flat_map(|(vertices, _)| vertices) {
        for axis in 0..2 {
            min[axis] = min[axis].min(vertex.xy[axis]);
            max[axis] = max[axis].max(vertex.xy[axis]);
        }
    }
    if !(min[0].is_finite() && max[0].is_finite() && min[1].is_finite() && max[1].is_finite()) {
        return None;
    }
    let origin = [min[0].floor(), max[1].ceil()];
    let width = (((max[0].ceil() - origin[0]) * RASTER_SCALE) as u32).max(1);
    let height = (((origin[1] - min[1].floor()) * RASTER_SCALE) as u32).max(1);
    if width > MAX_RASTER_SIDE || height > MAX_RASTER_SIDE {
        return None;
    }
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    let mut depth = vec![f32::INFINITY; width as usize * height as usize];
    let to_raster = |xy: [f32; 2]| {
        [
            (xy[0] - origin[0]) * RASTER_SCALE,
            (origin[1] - xy[1]) * RASTER_SCALE,
        ]
    };
    for (vertices, texels) in triangles {
        let p = vertices.map(|vertex| to_raster(vertex.xy));
        let area = edge(p[0], p[1], p[2]);
        if area.abs() < 1e-6 {
            continue;
        }
        let x0 = p
            .iter()
            .map(|q| q[0])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as u32;
        let x1 = (p
            .iter()
            .map(|q| q[0])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil() as u32)
            .min(width);
        let y0 = p
            .iter()
            .map(|q| q[1])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as u32;
        let y1 = (p
            .iter()
            .map(|q| q[1])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil() as u32)
            .min(height);
        let sign = if area < 0.0 { -1.0 } else { 1.0 };
        let sides = [(p[1], p[2]), (p[2], p[0]), (p[0], p[1])];
        for y in y0..y1 {
            // Columns far from the triangle fail the weight test anyway.
            let (mut from, mut to) = (i64::from(x0), i64::from(x1));
            let py = y as f32 + 0.5;
            let at = x0 as f32 + 0.5;
            for (a, b) in sides.into_iter().filter(|_| narrow) {
                let f0 = sign * edge(a, b, [at, py]);
                let slope = sign * edge(a, b, [at + 1.0, py]) - f0;
                if slope > 1e-12 {
                    from = from.max(i64::from(x0) + (-f0 / slope).floor() as i64 - 2);
                } else if slope < -1e-12 {
                    to = to.min(i64::from(x0) + (-f0 / slope).ceil() as i64 + 3);
                }
            }
            for x in (from.max(0) as u32)..(to.max(0) as u32).min(x1) {
                let c = [x as f32 + 0.5, y as f32 + 0.5];
                let w = [
                    edge(p[1], p[2], c) / area,
                    edge(p[2], p[0], c) / area,
                    edge(p[0], p[1], c) / area,
                ];
                if w.iter().any(|weight| *weight < 0.0) {
                    continue;
                }
                let z =
                    w[0] * vertices[0].depth + w[1] * vertices[1].depth + w[2] * vertices[2].depth;
                let at = y as usize * width as usize + x as usize;
                if z >= depth[at] {
                    continue;
                }
                let texel = match texels {
                    Some(texels) => {
                        let uv = [0, 1].map(|axis| {
                            w[0] * vertices[0].uv[axis]
                                + w[1] * vertices[1].uv[axis]
                                + w[2] * vertices[2].uv[axis]
                        });
                        texels.sample(uv)
                    }
                    None => [160, 160, 160, 255],
                };
                let shade = [0, 1, 2, 3].map(|channel| {
                    w[0] * vertices[0].color[channel]
                        + w[1] * vertices[1].color[channel]
                        + w[2] * vertices[2].color[channel]
                });
                let Some(pixel) = fragment(texel, shade) else {
                    continue;
                };
                depth[at] = z;
                rgba[at * 4..at * 4 + 4].copy_from_slice(&pixel);
            }
        }
    }
    let nearest = depth.iter().copied().fold(f32::INFINITY, f32::min);
    nearest.is_finite().then_some(Raster {
        width,
        height,
        rgba,
        origin,
        depth: nearest,
    })
}

fn edge(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Model space to field space: root scale `(-1, -1, 1)`, then the facing
/// turn about Unity's up axis, then the actor position.
pub fn place_vertex(vertex: [f32; 3], facing: u8, position: [f32; 3]) -> [f32; 3] {
    let scaled = [-vertex[0], -vertex[1], vertex[2]];
    let angle = (f32::from(facing) * 360.0 / 256.0).to_radians();
    let (sin, cos) = angle.sin_cos();
    [
        position[0] + scaled[0] * cos + scaled[2] * sin,
        position[1] + scaled[1],
        position[2] - scaled[0] * sin + scaled[2] * cos,
    ]
}

/// Models and clips loaded so far, keyed by script id; `None` marks an id
/// that does not load, so it is not retried.
pub struct ActorModels {
    models_bundle: PathBuf,
    clips_bundle: PathBuf,
    models: HashMap<u16, Option<AssembledModel>>,
    clips: HashMap<u16, Option<AnimationClip>>,
    configs_bundle: PathBuf,
    /// Every field's map config, loaded on the first field.
    configs: Option<HashMap<String, MapConfig>>,
    /// The current field's map config.
    config: Option<MapConfig>,
    shadow_plate: Option<crate::field_light::ShadowPlate>,
    /// The texture animations (blinking eyes) of each loaded model, one state per model.
    tex_anims: HashMap<u16, tantalus_assets::pc::geo_tex_anim::TexAnimPlayer>,
    /// The texture animations of each field actor (`GeoTexAnim` is a component of the actor's
    /// own model object): the model it was made for, its state, and how many of the actor's
    /// script commands it has run.
    actor_tex_anims: HashMap<u64, ActorTexAnim>,
}

struct ActorTexAnim {
    model: u16,
    player: tantalus_assets::pc::geo_tex_anim::TexAnimPlayer,
    applied: usize,
}

impl ActorModels {
    /// `streaming_assets` holds `p0data4.bin`, `p0data5.bin` and
    /// `p0data7.bin`.
    pub fn new(streaming_assets: &Path) -> Self {
        Self {
            models_bundle: streaming_assets.join("p0data4.bin"),
            clips_bundle: streaming_assets.join("p0data5.bin"),
            models: HashMap::new(),
            clips: HashMap::new(),
            configs_bundle: streaming_assets.join("p0data7.bin"),
            configs: None,
            config: None,
            shadow_plate: None,
            tex_anims: HashMap::new(),
            actor_tex_anims: HashMap::new(),
        }
    }

    /// The redrawn main texture of material `part` of `model`, when a world model's animation
    /// has drawn on it.
    pub fn model_canvas(&self, model: u16, part: usize) -> Option<&[u8]> {
        self.tex_anims.get(&model)?.canvas(part)
    }

    /// Runs each field actor's texture animations up to game tick `tick`, with the commands its
    /// scripts gave (`EventEngine.DoEventCode` `TEXPLAY`, `TEXPLAY1`, `TEXSTOP`; the blink
    /// restarts itself as `FieldMap.Update` does). `clock` is the field's own tick count,
    /// `commands` are stamped with it.
    pub fn advance_actor_tex_anims(
        &mut self,
        tick: u64,
        poses: &std::collections::BTreeMap<u64, ActorPose>,
        commands: &std::collections::BTreeMap<
            tantalus_contracts::EntityId,
            Vec<tantalus_game::field::world::TexAnimCommand>,
        >,
        clock: u64,
    ) {
        use tantalus_assets::pc::geo_tex_anim::{TexAnimPlayer, material_textures};
        use tantalus_game::field::world::TexAnimOp;
        self.actor_tex_anims.retain(|id, _| poses.contains_key(id));
        for (id, pose) in poses {
            let Some(model_id) = pose.model else {
                continue;
            };
            let Some(Some(model)) = self.models.get(&model_id) else {
                continue;
            };
            let Some(table) = &model.tex_anim else {
                continue;
            };
            let fresh = || {
                let mut player = TexAnimPlayer::new(table.clone());
                player.field_blink = true;
                ActorTexAnim {
                    model: model_id,
                    player,
                    applied: 0,
                }
            };
            let state = self.actor_tex_anims.entry(*id).or_insert_with(fresh);
            if state.model != model_id {
                *state = fresh();
            }
            let textures = material_textures(model);
            let own = commands
                .get(&tantalus_contracts::EntityId::new(*id))
                .map_or(&[][..], Vec::as_slice);
            // A field that restarted its clock starts the list over.
            if state.applied > own.len() {
                state.applied = 0;
            }
            for command in &own[state.applied..] {
                let at = (command.tick + tick).saturating_sub(clock);
                state.player.advance_to(at.min(tick), &textures);
                let index = usize::try_from(command.index).unwrap_or(usize::MAX);
                match command.op {
                    TexAnimOp::Play => state.player.anim.play(index),
                    TexAnimOp::PlayOnce => state.player.anim.play_once(index),
                    TexAnimOp::Stop => state.player.anim.stop(index),
                }
            }
            state.applied = own.len();
            state.player.advance_to(tick, &textures);
        }
    }

    /// Runs the loaded models' texture animations up to field tick `tick`
    /// (`GeoTexAnim.HonoUpdate`: `geoTexAnimService` once a tick, 30 a second).
    pub fn advance_tex_anims(&mut self, tick: u64) {
        use tantalus_assets::pc::geo_tex_anim::{TexAnimPlayer, material_textures};
        for (id, model) in &self.models {
            let Some(model) = model else {
                continue;
            };
            let Some(table) = &model.tex_anim else {
                continue;
            };
            if !self.tex_anims.contains_key(id)
                && std::env::var_os("TANTALUS_TEXANIM_LOG").is_some()
            {
                eprintln!(
                    "texanim model {id}: {} anims main {:?} sub {:?} textures {:?} headers {:?}",
                    table.anims.len(),
                    table.main_index,
                    table.sub_index,
                    tantalus_assets::pc::geo_tex_anim::material_textures(model)
                        .iter()
                        .map(|t| t.map(|t| (t.width, t.height)))
                        .collect::<Vec<_>>(),
                    table
                        .anims
                        .iter()
                        .map(|a| (
                            a.flags,
                            a.num_frames,
                            a.rate,
                            a.rand_min,
                            a.rand_range,
                            a.target
                        ))
                        .collect::<Vec<_>>(),
                );
            }
            let player = self.tex_anims.entry(*id).or_insert_with(|| {
                let mut player = TexAnimPlayer::new(table.clone());
                player.field_blink = true;
                player
            });
            player.advance_to(tick, &material_textures(model));
            if std::env::var_os("TANTALUS_TEXANIM_LOG").is_some() {
                eprintln!(
                    "texanim model {id} tick {tick}: frames {:?} canvases {:?}",
                    player
                        .anim
                        .anims
                        .iter()
                        .map(|a| (a.flags, a.frame >> 12))
                        .collect::<Vec<_>>(),
                    (0..4)
                        .map(|i| player.canvas(i).is_some())
                        .collect::<Vec<_>>()
                );
            }
        }
    }

    /// Selects the map config of `field` (its `EVT_...` name), loading
    /// every config and the shadow texture the first time. Returns one line
    /// per file that failed to load.
    pub fn prepare_field(&mut self, field: Option<&str>) -> Vec<String> {
        let mut problems = Vec::new();
        if self.configs.is_none() {
            let (configs, plate) = crate::field_light::load_field_light(&self.configs_bundle)
                .unwrap_or_else(|error| {
                    problems.push(format!("map configs: {error}"));
                    (HashMap::new(), None)
                });
            if plate.is_none() {
                problems.push("no shadow_plate texture in p0data7.bin".into());
            }
            self.configs = Some(configs);
            self.shadow_plate = plate;
        }
        self.config = field.and_then(|field| {
            self.configs
                .as_ref()?
                .get(&field.to_ascii_lowercase())
                .cloned()
        });
        problems
    }

    /// Replays the field scripts' `MCF_*` light changes on the current
    /// map config (after [`Self::prepare_field`], which resets it).
    pub fn apply_light_changes(&mut self, changes: &[tantalus_game::field::world::LightChange]) {
        crate::field_light::apply_light_changes(&mut self.config, changes);
    }

    /// The current field's light for `pose`.
    pub fn light(&self, pose: &ActorPose) -> crate::field_light::ActorLight {
        pose.model.map_or_else(Default::default, |model| {
            crate::field_light::actor_light_scripted(
                self.config.as_ref(),
                model,
                pose.floor,
                &pose.script,
            )
        })
    }

    /// `pose`'s shadow quad (`FieldMapActor.CreateShadowMesh`) projected
    /// to the field, or `None` when the actor casts none.
    pub fn shadow_quad(
        &self,
        camera: &LegacyFieldCamera,
        position: [f32; 3],
        pose: &ActorPose,
    ) -> Option<ShadowQuad> {
        let shadow = self.light(pose).shadow?;
        self.shadow_plate.as_ref()?;
        // `FieldMapActor`: the shadow sits at the actor plus `xOffset`,
        // `zOffset` (`SHADOWOFFSET`).
        let center = [
            position[0] + shadow.offset[0],
            position[1] + crate::field_light::SHADOW_HEIGHT_OFFSET,
            position[2] + shadow.offset[1],
        ];
        let corner = |x: f32, z: f32| {
            let field = [
                center[0] + x * shadow.half[0],
                center[1],
                center[2] + z * shadow.half[1],
            ];
            let (xy, depth) = project_field_point_depth(camera, field)?;
            Some(ScreenVertex {
                xy,
                depth,
                uv: [(x + 1.0) / 2.0, (z + 1.0) / 2.0],
                color: [1.0; 4],
            })
        };
        Some(ShadowQuad {
            corners: [
                corner(-1.0, -1.0)?,
                corner(1.0, -1.0)?,
                corner(1.0, 1.0)?,
                corner(-1.0, 1.0)?,
            ],
            strength: shadow.strength,
        })
    }

    /// The shadow texture: width, height and RGBA rows top-down.
    pub fn shadow_plate(&self) -> Option<&crate::field_light::ShadowPlate> {
        self.shadow_plate.as_ref()
    }

    /// Rasterizes `pose`'s shadow quad baked for a reverse-subtract draw
    /// (the CPU path; the GPU path draws [`Self::shadow_quad`]), or `None`
    /// when the actor casts none.
    pub fn render_shadow(
        &self,
        camera: &LegacyFieldCamera,
        position: [f32; 3],
        pose: &ActorPose,
    ) -> Option<Raster> {
        let quad = self.shadow_quad(camera, position, pose)?;
        let (width, height, rgba) = self.shadow_plate.as_ref()?;
        let texels = Texels {
            width: *width,
            height: *height,
            rgba,
        };
        let q = quad.corners;
        rasterize_with(
            &[
                ([q[2], q[1], q[0]], Some(texels)),
                ([q[3], q[2], q[0]], Some(texels)),
            ],
            |texel, _| {
                let pixel = crate::field_light::shadow_texel(texel, quad.strength);
                (pixel[3] != 0).then_some(pixel)
            },
        )
    }

    /// Loads the models and clips `poses` name that are not loaded yet.
    /// Returns one line per bundle that failed to open.
    pub fn prepare<'p>(&mut self, poses: impl IntoIterator<Item = &'p ActorPose>) -> Vec<String> {
        let mut want_models = BTreeSet::new();
        let mut want_clips = BTreeSet::new();
        for pose in poses {
            if let Some(model) = pose.model.filter(|model| !self.models.contains_key(model)) {
                want_models.insert(model);
            }
            if let Some(clip) = pose.animation.filter(|clip| !self.clips.contains_key(clip)) {
                want_clips.insert(clip);
            }
        }
        let mut problems = Vec::new();
        if !want_models.is_empty() {
            let loaded = with_store(&self.models_bundle, |store| {
                let entries = store.container().map_err(|error| error.to_string())?;
                let catalog = ModelCatalog::from_container(&entries);
                Ok(want_models
                    .iter()
                    .map(|id| {
                        let model = model_dirs(u32::from(*id), &catalog)
                            .into_iter()
                            .find_map(|dir| {
                                let root = *dir.roots.first()?;
                                assemble_model(
                                    store,
                                    &dir.label,
                                    root,
                                    AssembleOptions {
                                        decode_textures: true,
                                    },
                                )
                                .ok()
                            })
                            .map(|mut model| {
                                model.tex_anim = tantalus_assets::pc::geo_tex_anim::load(
                                    store,
                                    &entries,
                                    u32::from(*id),
                                );
                                model
                            });
                        (*id, model)
                    })
                    .collect::<Vec<_>>())
            });
            match loaded {
                Ok(loaded) => self.models.extend(loaded),
                Err(error) => {
                    problems.push(format!("models: {error}"));
                    self.models.extend(want_models.iter().map(|id| (*id, None)));
                }
            }
        }
        if !want_clips.is_empty() {
            let loaded = with_store(&self.clips_bundle, |store| {
                let entries = store.container().map_err(|error| error.to_string())?;
                let (clips, _) = clip_entries(&entries);
                let by_name: HashMap<(u32, &str), i64> = clips
                    .iter()
                    .map(|clip| ((clip.geo_id, clip.name.as_str()), clip.path_id))
                    .collect();
                Ok(want_clips
                    .iter()
                    .map(|id| {
                        let resolved = resolve_animation(*id, None, &HashMap::new());
                        let clip = resolved
                            .folder_geo_id
                            .zip(resolved.clip_key)
                            .and_then(|(folder, key)| {
                                by_name.get(&(folder, key.to_string().as_str())).copied()
                            })
                            .and_then(|path_id| store.clip(path_id).ok().flatten());
                        (*id, clip)
                    })
                    .collect::<Vec<_>>())
            });
            match loaded {
                Ok(loaded) => self.clips.extend(loaded),
                Err(error) => {
                    problems.push(format!("clips: {error}"));
                    self.clips.extend(want_clips.iter().map(|id| (*id, None)));
                }
            }
        }
        problems
    }

    /// The skinned vertices of `model` at `time` seconds into clip
    /// `animation` (the rest pose without a clip), per part.
    pub fn skinned(
        &self,
        model: u16,
        animation: Option<u16>,
        time: f32,
    ) -> Option<(&AssembledModel, Vec<Vec<[f32; 3]>>)> {
        let assembled = self.models.get(&model)?.as_ref()?;
        let clip = animation.and_then(|animation| self.clips.get(&animation)?.as_ref());
        let poses = match clip {
            Some(clip) => {
                let length = clip_length(clip);
                let time = if length > 0.0 {
                    time.rem_euclid(length)
                } else {
                    0.0
                };
                sample_clip(
                    assembled,
                    clip,
                    &bind_clip(assembled, clip),
                    time,
                    Interpolation::Hermite,
                )
            }
            None => rest_pose(assembled),
        };
        Some((assembled, skin_model(assembled, &poses)))
    }

    /// Whether `model` is loaded and assembled.
    pub fn has_model(&self, model: u16) -> bool {
        self.models.get(&model).is_some_and(Option::is_some)
    }

    /// Rasterizes one actor, or `None` without a loaded model or when it
    /// projects to nothing.
    pub fn render(
        &self,
        camera: &LegacyFieldCamera,
        position: [f32; 3],
        pose: &ActorPose,
    ) -> Option<Raster> {
        self.render_carrying(camera, position, pose, &[])
    }

    /// Where bone `bone` (`bone<NNN>`) of `pose`'s model is in the field, for an actor standing
    /// at `position` (`FieldSPSSystem.GenerateSPS`: `boneTran.position`).
    pub fn bone_position(
        &self,
        pose: &ActorPose,
        position: [f32; 3],
        bone: i32,
    ) -> Option<[f32; 3]> {
        let model = self.models.get(&pose.model?)?.as_ref()?;
        let poses = self.node_poses(model, pose);
        let world = world_matrices(&model.nodes, &poses);
        let name = format!("bone{bone:03}");
        let index = model.nodes.iter().position(|node| node.name == name)?;
        Some(place_vertex(
            transform_point(&world[index], [0.0; 3]),
            pose.facing,
            position,
        ))
    }

    /// The node poses of `pose`'s model: the clip at its time, or the rest pose.
    fn node_poses(&self, model: &AssembledModel, pose: &ActorPose) -> Vec<NodePose> {
        let clip = pose
            .animation
            .and_then(|animation| self.clips.get(&animation)?.as_ref());
        match clip {
            Some(clip) => {
                let length = clip_length(clip);
                let time = if length > 0.0 {
                    pose.time.rem_euclid(length)
                } else {
                    0.0
                };
                sample_clip(
                    model,
                    clip,
                    &bind_clip(model, clip),
                    time,
                    Interpolation::Hermite,
                )
            }
            None => rest_pose(model),
        }
    }

    /// Projects every shown triangle of `pose`'s model. `place` carries a skinned vertex
    /// (model space) to the field.
    fn collect_triangles<'a>(
        &'a self,
        camera: &LegacyFieldCamera,
        model: &'a AssembledModel,
        pose: &ActorPose,
        skinned: &[Vec<[f32; 3]>],
        place: &dyn Fn([f32; 3]) -> [f32; 3],
        triangles: &mut Vec<([ScreenVertex; 3], Option<Texels<'a>>)>,
    ) {
        let tint = self.light(pose).tint;
        for (part_index, (part, vertices)) in model.parts.iter().zip(skinned).enumerate() {
            if under_battle_model(model, part.node)
                || mesh_index(model, part.node)
                    .is_some_and(|mesh| mesh < 32 && pose.hidden_meshes & (1 << mesh) != 0)
                || hair_hidden(model, part.node, pose)
            {
                continue;
            }
            let projected: Vec<Option<ScreenVertex>> = vertices
                .iter()
                .enumerate()
                .map(|(index, vertex)| {
                    let vertex = pose.scale.map_or(*vertex, |scale| {
                        [
                            vertex[0] * scale[0],
                            vertex[1] * scale[1],
                            vertex[2] * scale[2],
                        ]
                    });
                    let (xy, depth) = project_field_point_depth(camera, place(vertex))?;
                    Some(ScreenVertex {
                        xy,
                        depth,
                        uv: part.uvs.get(index).copied().unwrap_or_default(),
                        color: {
                            let [r, g, b, a] = part.colors.get(index).copied().unwrap_or([1.0; 4]);
                            [r * tint[0], g * tint[1], b * tint[2], a]
                        },
                    })
                })
                .collect();
            for (submesh_index, submesh) in part.submeshes.iter().enumerate() {
                if submesh.topology != TOPOLOGY_TRIANGLES {
                    continue;
                }
                let mut texels = submesh.material.as_ref().and_then(|material| {
                    material.textures.iter().find_map(|texture| {
                        let rgba = texture.rgba.as_deref()?;
                        let width = usize::try_from(texture.width).ok()?;
                        let height = usize::try_from(texture.height).ok()?;
                        (width > 0 && height > 0 && rgba.len() >= width * height * 4).then_some(
                            Texels {
                                width,
                                height,
                                rgba,
                            },
                        )
                    })
                });
                // `GeoTexAnim`: the renderer's own material (its first) shows the redrawn texture.
                if let (0, Some(texels)) = (submesh_index, texels.as_mut())
                    && let Some(canvas) = self
                        .actor_tex_anims
                        .get(&pose.actor)
                        .filter(|state| Some(state.model) == pose.model)
                        .and_then(|state| state.player.canvas(part_index))
                    && canvas.len() >= texels.width * texels.height * 4
                {
                    texels.rgba = canvas;
                }
                let first = submesh.first_index as usize;
                let end = first + submesh.index_count as usize;
                let Some(indices) = part.indices.get(first..end) else {
                    continue;
                };
                for corner in indices.chunks_exact(3) {
                    let vertex = |at: u32| projected.get(at as usize).copied().flatten();
                    if let (Some(a), Some(b), Some(c)) =
                        (vertex(corner[0]), vertex(corner[1]), vertex(corner[2]))
                    {
                        triangles.push(([a, b, c], texels));
                    }
                }
            }
        }
    }

    /// [`Self::render`] with the objects other scripts attached (`AttachObject`) to its bones
    /// drawn with it: each rides its carrier's `bone<NNN>` node, moved by the attach offset
    /// (`geo.geoAttach`, `FieldMapActor.GeoAttach`).
    pub fn render_carrying(
        &self,
        camera: &LegacyFieldCamera,
        position: [f32; 3],
        pose: &ActorPose,
        carried: &[CarriedPose],
    ) -> Option<Raster> {
        let triangles = self.carrying_triangles(camera, position, pose, carried)?;
        let t0 = std::time::Instant::now();
        let out = rasterize(&triangles);
        crate::live::bench::record("a_raster", t0.elapsed().as_secs_f64() * 1000.0);
        crate::live::bench::record("a_tris", triangles.len() as f64);
        out
    }

    /// The screen triangles of [`Self::render_carrying`], before they are
    /// drawn (by [`rasterize`] or the GPU path).
    pub fn carrying_triangles(
        &self,
        camera: &LegacyFieldCamera,
        position: [f32; 3],
        pose: &ActorPose,
        carried: &[CarriedPose],
    ) -> Option<Vec<Triangle<'_>>> {
        let model = self.models.get(&pose.model?)?.as_ref()?;
        let t0 = std::time::Instant::now();
        let poses = self.node_poses(model, pose);
        let skinned = skin_model(model, &poses);
        crate::live::bench::record("a_skin", t0.elapsed().as_secs_f64() * 1000.0);
        let t0 = std::time::Instant::now();
        let own = |vertex: [f32; 3]| place_vertex(vertex, pose.facing, position);
        let mut triangles = Vec::new();
        self.collect_triangles(camera, model, pose, &skinned, &own, &mut triangles);
        if !carried.is_empty() {
            let world = world_matrices(&model.nodes, &poses);
            for item in carried {
                let Some(carried_model) = item
                    .pose
                    .model
                    .and_then(|id| self.models.get(&id)?.as_ref())
                else {
                    continue;
                };
                let name = format!("bone{:03}", item.bone);
                let Some(bone) = model.nodes.iter().position(|node| node.name == name) else {
                    continue;
                };
                let bone_world = world[bone];
                // `GeoAttachOffset` stores the script offset negated, in the bone's space.
                let offset = item.offset.map(|value| -(value as f32));
                let carried_poses = self.node_poses(carried_model, &item.pose);
                let carried_skinned = skin_model(carried_model, &carried_poses);
                let on_bone = |vertex: [f32; 3]| {
                    let local = [
                        vertex[0] + offset[0],
                        vertex[1] + offset[1],
                        vertex[2] + offset[2],
                    ];
                    own(transform_point(&bone_world, local))
                };
                self.collect_triangles(
                    camera,
                    carried_model,
                    &item.pose,
                    &carried_skinned,
                    &on_bone,
                    &mut triangles,
                );
            }
        }
        crate::live::bench::record("a_collect", t0.elapsed().as_secs_f64() * 1000.0);
        Some(triangles)
    }
}

/// An actor drawn on another's bone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarriedPose {
    pub pose: ActorPose,
    /// The carrier's bone index (`bone<NNN>`).
    pub bone: i32,
    /// `AttachObjectOffset` x, y, z.
    pub offset: [i32; 3],
}

/// `ModelFactory.CreateModel` disables every renderer under the
/// `battle_model` child outside battle.
fn under_battle_model(model: &AssembledModel, node: usize) -> bool {
    let mut at = Some(node);
    while let Some(index) = at {
        let Some(node) = model.nodes.get(index) else {
            return false;
        };
        if node.name == "battle_model" {
            return true;
        }
        at = node.parent;
    }
    false
}

/// Whether `node` is part of the hair Garnet's model does not show.
///
/// Her model carries `long_hair` and `short_hair`; `ModelFactory` disables
/// the one that does not match the story (`garnetShortHair`), and
/// `PosObj.SetIsEnabledMeshRenderer` points `MESHHIDE`/`MESHSHOW` of mesh 1
/// at the one that is shown.
fn hair_hidden(model: &AssembledModel, node: usize, pose: &ActorPose) -> bool {
    let under = |name: &str| {
        let mut at = Some(node);
        while let Some(index) = at {
            let Some(found) = model.nodes.get(index) else {
                return false;
            };
            if found.name == name {
                return true;
            }
            at = found.parent;
        }
        false
    };
    let (shown, other) = if pose.short_hair {
        ("short_hair", "long_hair")
    } else {
        ("long_hair", "short_hair")
    };
    under(other) || (under(shown) && pose.hidden_meshes & 2 != 0)
}

/// The `<i>` of the `mesh<i>` child of the model root that holds `node`
/// (`PosObj.SetIsEnabledMeshRenderer` toggles those children).
fn mesh_index(model: &AssembledModel, node: usize) -> Option<u32> {
    let mut at = node;
    loop {
        let parent = model.nodes.get(at)?.parent?;
        if model.nodes.get(parent)?.parent.is_none() {
            return model.nodes[at].name.strip_prefix("mesh")?.parse().ok();
        }
        at = parent;
    }
}

/// The last key time over every curve.
fn clip_length(clip: &AnimationClip) -> f32 {
    let last = |times: &mut dyn Iterator<Item = f32>| times.fold(0.0f32, f32::max);
    last(
        &mut clip
            .rotation_curves
            .iter()
            .filter_map(|curve| curve.keys.last().map(|key| key.time)),
    )
    .max(last(
        &mut clip
            .position_curves
            .iter()
            .filter_map(|curve| curve.keys.last().map(|key| key.time)),
    ))
    .max(last(
        &mut clip
            .scale_curves
            .iter()
            .filter_map(|curve| curve.keys.last().map(|key| key.time)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(x: f32, y: f32, depth: f32, uv: [f32; 2]) -> ScreenVertex {
        ScreenVertex {
            xy: [x, y],
            depth,
            uv,
            color: [1.0; 4],
        }
    }

    #[test]
    fn vertex_colors_multiply_the_texel_and_their_alpha_cuts() {
        let rgba = [200u8, 100, 50, 255];
        let texels = Texels {
            width: 1,
            height: 1,
            rgba: &rgba,
        };
        let shaded = |color: [f32; 4]| {
            let tri = [
                ScreenVertex {
                    color,
                    ..vertex(0.0, 0.0, 1.0, [0.5, 0.5])
                },
                ScreenVertex {
                    color,
                    ..vertex(4.0, 0.0, 1.0, [0.5, 0.5])
                },
                ScreenVertex {
                    color,
                    ..vertex(0.0, 4.0, 1.0, [0.5, 0.5])
                },
            ];
            rasterize(&[(tri, Some(texels))]).map(|raster| pixel(&raster, 1, raster.height - 2))
        };
        assert_eq!(shaded([1.0; 4]), Some([200, 100, 50, 255]));
        assert_eq!(shaded([0.5, 0.5, 0.5, 1.0]), Some([100, 50, 25, 255]));
        // Texel alpha 1 times vertex alpha 0.4 is under the 0.5 cut.
        assert_eq!(shaded([1.0, 1.0, 1.0, 0.4]), None);
    }

    fn pixel(raster: &Raster, x: u32, y: u32) -> [u8; 4] {
        let at = (y * raster.width + x) as usize * 4;
        raster.rgba[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn a_textured_triangle_samples_its_texels() {
        // 2x1 texture: left red, right blue.
        let texels = Texels {
            width: 2,
            height: 1,
            rgba: &[255, 0, 0, 255, 0, 0, 255, 255],
        };
        let square = [
            vertex(0.0, 0.0, 10.0, [0.0, 0.0]),
            vertex(4.0, 0.0, 10.0, [1.0, 0.0]),
            vertex(4.0, 4.0, 10.0, [1.0, 1.0]),
        ];
        let other = [
            vertex(0.0, 0.0, 10.0, [0.0, 0.0]),
            vertex(4.0, 4.0, 10.0, [1.0, 1.0]),
            vertex(0.0, 4.0, 10.0, [0.0, 1.0]),
        ];
        let raster = rasterize(&[(square, Some(texels)), (other, Some(texels))]).unwrap();
        assert_eq!((raster.width, raster.height), (16, 16));
        assert_eq!(raster.origin, [0.0, 4.0]);
        assert_eq!(pixel(&raster, 1, 8), [255, 0, 0, 255]);
        assert_eq!(pixel(&raster, 14, 8), [0, 0, 255, 255]);
        assert_eq!(raster.placement(), ([2.0, 2.0], [4.0, 4.0]));
    }

    #[test]
    fn the_nearer_triangle_wins_whatever_the_draw_order() {
        let tri = |depth: f32| {
            [
                vertex(0.0, 0.0, depth, [0.0, 0.0]),
                vertex(4.0, 0.0, depth, [0.0, 0.0]),
                vertex(0.0, 4.0, depth, [0.0, 0.0]),
            ]
        };
        let red = Texels {
            width: 1,
            height: 1,
            rgba: &[255, 0, 0, 255],
        };
        let green = Texels {
            width: 1,
            height: 1,
            rgba: &[0, 255, 0, 255],
        };
        for order in [[(5.0, red), (9.0, green)], [(9.0, green), (5.0, red)]] {
            let triangles: Vec<_> = order
                .iter()
                .map(|(depth, texels)| (tri(*depth), Some(*texels)))
                .collect();
            let raster = rasterize(&triangles).unwrap();
            assert_eq!(pixel(&raster, 2, 13), [255, 0, 0, 255]);
            assert_eq!(raster.depth, 5.0);
        }
    }

    #[test]
    fn cut_out_texels_leave_the_pixel_empty() {
        let clear = Texels {
            width: 1,
            height: 1,
            rgba: &[255, 255, 255, 0],
        };
        let tri = [
            vertex(0.0, 0.0, 1.0, [0.0, 0.0]),
            vertex(4.0, 0.0, 1.0, [0.0, 0.0]),
            vertex(0.0, 4.0, 1.0, [0.0, 0.0]),
        ];
        assert!(rasterize(&[(tri, Some(clear))]).is_none());
    }

    #[test]
    fn placement_flips_the_model_and_turns_it_by_the_facing() {
        // A point one unit up and one unit forward of the model origin.
        let up_forward = [0.0, 1.0, 1.0];
        assert_eq!(
            place_vertex(up_forward, 0, [10.0, 0.0, 0.0]),
            [10.0, -1.0, 1.0]
        );
        let quarter = place_vertex(up_forward, 64, [0.0; 3]);
        assert!((quarter[0] - 1.0).abs() < 1e-5 && quarter[2].abs() < 1e-5);
    }
}

#[cfg(test)]
mod narrowing_tests {
    use super::*;

    #[test]
    fn narrowed_rows_match_the_bounding_box_scan() {
        let mut seed = 0x9e37_79b9_u32;
        let mut next = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / 16_777_216.0
        };
        let triangles: Vec<_> = (0..300)
            .map(|_| {
                let mut vertex = || ScreenVertex {
                    xy: [next() * 40.0, next() * 50.0],
                    depth: next() * 10.0,
                    uv: [0.0, 0.0],
                    color: [next(), next(), next(), 1.0],
                };
                ([vertex(), vertex(), vertex()], None)
            })
            .collect();
        let fragment = |texel: [u8; 4], shade: [f32; 4]| {
            Some([
                (shade[0].clamp(0.0, 1.0) * f32::from(texel[0])) as u8,
                (shade[1].clamp(0.0, 1.0) * f32::from(texel[1])) as u8,
                (shade[2].clamp(0.0, 1.0) * f32::from(texel[2])) as u8,
                255,
            ])
        };
        let narrow = rasterize_rows(&triangles, fragment, true).expect("raster");
        let full = rasterize_rows(&triangles, fragment, false).expect("raster");
        assert_eq!(narrow.rgba, full.rgba);
    }
}
