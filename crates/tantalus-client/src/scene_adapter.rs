use crate::animation_adapter::AnimationAdapter;
use crate::legacy_compositor::LegacyCompositor;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use tantalus_contracts::{
    ContentId, ContentRevision, EntityId, FieldSceneDescription, FieldSpritePresentation, Point3,
    PresentationSnapshot, ScenePresentationMode,
};

#[derive(Debug, Clone, Default)]
pub struct ResolvedAssets {
    pub content_revision: ContentRevision,
    pub images: BTreeMap<ContentId, Handle<Image>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientError {
    pub context: String,
}

impl ClientError {
    pub fn new(context: impl Into<String>) -> Self {
        Self {
            context: context.into(),
        }
    }
}

impl Display for ClientError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.context)
    }
}

impl Error for ClientError {}

pub struct ClientSceneAdapter {
    app: App,
    scene_entities: Vec<Entity>,
    render_target: Option<RenderTarget>,
    mode: ScenePresentationMode,
}

impl Default for ClientSceneAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientSceneAdapter {
    pub fn new() -> Self {
        Self::with_app(App::new())
    }

    pub fn with_app(app: App) -> Self {
        Self {
            app,
            scene_entities: Vec::new(),
            render_target: None,
            mode: ScenePresentationMode::Layered2d,
        }
    }

    pub fn app(&self) -> &App {
        &self.app
    }

    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    pub fn set_render_target(&mut self, target: RenderTarget) {
        self.render_target = Some(target);
    }

    pub fn apply(
        &mut self,
        snapshot: &PresentationSnapshot,
        _assets: &ResolvedAssets,
    ) -> Result<(), ClientError> {
        validate_snapshot(snapshot)?;
        self.clear_scene();
        self.mode = snapshot.mode;

        for layer in LegacyCompositor::ordered_layers(snapshot) {
            self.spawn_layer(layer.id, layer.position, layer.size, layer.color);
        }
        for actor in &snapshot.actors {
            self.spawn_actor(
                snapshot,
                actor.entity,
                actor.position,
                actor.size,
                actor.color,
                actor.animation.clone(),
            );
        }
        self.spawn_camera(snapshot);
        Ok(())
    }

    pub fn apply_field(
        &mut self,
        scene: &FieldSceneDescription,
        assets: &ResolvedAssets,
    ) -> Result<(), ClientError> {
        let sprites = field_sprites(scene, assets)?;
        self.clear_scene();
        self.mode = ScenePresentationMode::Layered2d;
        let target = self.render_target.clone();
        let entities = spawn_field_scene(self.app.world_mut(), scene, &sprites, assets, target);
        self.scene_entities.extend(entities);
        Ok(())
    }

    fn clear_scene(&mut self) {
        for entity in self.scene_entities.drain(..) {
            if let Ok(entity) = self.app.world_mut().get_entity_mut(entity) {
                entity.despawn();
            }
        }
    }

    fn spawn_layer(&mut self, id: ContentId, position: Point3, size: Point3, color: [u8; 4]) {
        let entity = if self.uses_3d() {
            let mesh = self
                .app
                .world_mut()
                .resource_mut::<Assets<Mesh>>()
                .add(Circle::new(to_units(size.x) / 2.0));
            let material = self
                .app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(to_color(color));
            self.app
                .world_mut()
                .spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    Transform::from_translation(to_vec3(position))
                        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                    LayerIdentity(id),
                ))
                .id()
        } else {
            self.app
                .world_mut()
                .spawn((
                    Sprite::from_color(
                        to_color(color),
                        Vec2::new(to_units(size.x), to_units(size.y)),
                    ),
                    Transform::from_translation(to_vec3(position)),
                    LayerIdentity(id),
                ))
                .id()
        };
        self.scene_entities.push(entity);
    }

    fn spawn_actor(
        &mut self,
        snapshot: &PresentationSnapshot,
        entity_id: EntityId,
        position: Point3,
        size: Point3,
        color: [u8; 4],
        animation: Option<ContentId>,
    ) {
        let entity = if self.uses_3d() {
            let mesh = self
                .app
                .world_mut()
                .resource_mut::<Assets<Mesh>>()
                .add(Cuboid::new(
                    to_units(size.x),
                    to_units(size.y),
                    to_units(size.z),
                ));
            let material = self
                .app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(to_color(color));
            self.app
                .world_mut()
                .spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    Transform::from_translation(to_vec3(position)),
                ))
                .id()
        } else {
            self.app
                .world_mut()
                .spawn((
                    Sprite::from_color(
                        to_color(color),
                        Vec2::new(to_units(size.x), to_units(size.y)),
                    ),
                    Transform::from_translation(to_vec3(position)),
                ))
                .id()
        };
        self.scene_entities.push(entity);
        self.app
            .world_mut()
            .entity_mut(entity)
            .insert(ActorIdentity(entity_id));
        if let Some(animation) = AnimationAdapter::map(animation, snapshot.tick) {
            self.app.world_mut().entity_mut(entity).insert(animation);
        }
    }

    fn spawn_camera(&mut self, snapshot: &PresentationSnapshot) {
        let transform = if self.uses_3d() {
            Transform::from_translation(to_vec3(snapshot.camera.position))
                .looking_at(to_vec3(snapshot.camera.target), Vec3::Y)
        } else {
            Transform::from_translation(to_vec3(snapshot.camera.position))
        };
        let camera = match (&self.render_target, self.uses_3d()) {
            (Some(target), true) => self
                .app
                .world_mut()
                .spawn((
                    Camera3d::default(),
                    transform,
                    target.clone(),
                    Tonemapping::None,
                ))
                .id(),
            (Some(target), false) => self
                .app
                .world_mut()
                .spawn((Camera2d, transform, target.clone()))
                .id(),
            (None, true) => self
                .app
                .world_mut()
                .spawn((Camera3d::default(), transform))
                .id(),
            (None, false) => self.app.world_mut().spawn((Camera2d, transform)).id(),
        };
        self.scene_entities.push(camera);

        if self.uses_3d() {
            self.scene_entities.push(
                self.app
                    .world_mut()
                    .spawn((
                        PointLight {
                            shadow_maps_enabled: true,
                            ..default()
                        },
                        Transform::from_xyz(4.0, 8.0, 4.0),
                    ))
                    .id(),
            );
        }
    }

    fn uses_3d(&self) -> bool {
        matches!(
            self.mode,
            ScenePresentationMode::Full3d | ScenePresentationMode::Hybrid
        )
    }
}

fn validate_snapshot(snapshot: &PresentationSnapshot) -> Result<(), ClientError> {
    let mut actors = BTreeSet::new();
    for actor in &snapshot.actors {
        if !actors.insert(actor.entity) {
            return Err(ClientError::new(format!(
                "duplicate presentation actor {}",
                actor.entity
            )));
        }
        validate_size(actor.size, "actor")?;
    }
    let mut layers = BTreeSet::new();
    for layer in &snapshot.layers {
        if !layers.insert(&layer.id) {
            return Err(ClientError::new(format!(
                "duplicate presentation layer {}",
                layer.id
            )));
        }
        validate_size(layer.size, "layer")?;
    }
    Ok(())
}

fn validate_size(size: Point3, owner: &str) -> Result<(), ClientError> {
    if size.x <= 0 || size.y <= 0 || size.z <= 0 {
        Err(ClientError::new(format!(
            "{owner} dimensions must be positive"
        )))
    } else {
        Ok(())
    }
}

fn to_units(value: i32) -> f32 {
    value as f32 / 1000.0
}

fn to_vec3(point: Point3) -> Vec3 {
    Vec3::new(to_units(point.x), to_units(point.y), to_units(point.z))
}

fn to_color(color: [u8; 4]) -> Color {
    Color::srgba_u8(color[0], color[1], color[2], color[3])
}

#[derive(Component)]
pub struct FieldSpriteIdentity {
    pub overlay: u32,
    pub sprite: u32,
}

#[derive(Component)]
pub struct LayerIdentity(pub ContentId);

#[derive(Component)]
pub struct ActorIdentity(pub EntityId);

/// The active sprites of the scene's camera, depth sorted, after checking
/// the scene: positive camera size, unique sprite identities, non-empty
/// atlas rectangles, resolved atlases.
pub fn field_sprites<'a>(
    scene: &'a FieldSceneDescription,
    assets: &ResolvedAssets,
) -> Result<Vec<&'a FieldSpritePresentation>, ClientError> {
    if scene.camera.size.contains(&0) {
        return Err(ClientError::new("field camera dimensions must be positive"));
    }
    let mut sprites = Vec::new();
    let mut identities = BTreeSet::new();
    for overlay in &scene.overlays {
        for sprite in &overlay.sprites {
            if !identities.insert((sprite.overlay_index, sprite.sprite_index)) {
                return Err(ClientError::new("duplicate field sprite identity"));
            }
            if sprite.atlas_rect[2] <= sprite.atlas_rect[0]
                || sprite.atlas_rect[3] <= sprite.atlas_rect[1]
            {
                return Err(ClientError::new(
                    "field sprite has an empty atlas rectangle",
                ));
            }
            if !assets.images.contains_key(&sprite.atlas) {
                return Err(ClientError::new(format!(
                    "field sprite atlas `{}` is not resolved",
                    sprite.atlas
                )));
            }
            // Inactive blocks are kept (hidden) for `BGLACTIVE` to show.
            if overlay.camera_index == scene.camera_index as u8 {
                sprites.push(sprite);
            }
        }
    }
    // Farthest first, as `field_composition` composes them.
    sprites.sort_by_key(|sprite| {
        (
            std::cmp::Reverse(sprite.depth),
            sprite.overlay_index,
            sprite.sprite_index,
        )
    });
    Ok(sprites)
}

/// Where the import put a background sprite, so the scripts' tile-block
/// changes apply over it each frame.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct FieldSpriteBase {
    pub translation: Vec3,
    pub active: bool,
}

impl FieldSpriteBase {
    fn visibility(&self) -> Visibility {
        if self.active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        }
    }
}

/// `FieldMap.charAimHeight`: `BgAttachService` aims at this height above
/// the player's position.
pub const CHAR_AIM_HEIGHT: f32 = 324.0;

/// The pixel offset of each changed tile block from its imported
/// position, and whether it shows. `player` is the player's position;
/// without one an attached block stays where it is.
pub fn overlay_placements(
    scene: &FieldSceneDescription,
    changes: &BTreeMap<u16, tantalus_game::field::world::OverlayChange>,
    player: Option<[f32; 3]>,
) -> BTreeMap<u32, (Option<bool>, [f32; 2])> {
    let mut placements = BTreeMap::new();
    for (index, change) in changes {
        let Some(overlay) = scene
            .overlays
            .iter()
            .find(|overlay| overlay.overlay_index == u32::from(*index))
        else {
            continue;
        };
        let imported = overlay.current_offset.map(f32::from);
        let mut current = change
            .origin
            .map(|origin| origin.map(f32::from))
            .unwrap_or(imported);
        let active = change.active.unwrap_or(overlay.flags & 2 != 0);
        // `BgAttachService`: an active attached block of this camera sits
        // at the player's projected aim point, less the scene offset and
        // the attach offset, plus the camera's view minimum.
        if let (Some([x, y]), Some(position), true) = (change.attached, player, active)
            && let Some([screen_x, screen_y]) = crate::projection::project_field_point(
                &scene.camera,
                [position[0], position[1] + CHAR_AIM_HEIGHT, position[2]],
            )
        {
            current = [
                (screen_x - f32::from(scene.scene_offset[0]) - f32::from(x)
                    + f32::from(scene.camera.viewport_x[0]))
                .trunc(),
                (-screen_y - f32::from(scene.scene_offset[1]) - f32::from(y)
                    + f32::from(scene.camera.viewport_y[0]))
                .trunc(),
            ];
        }
        placements.insert(
            overlay.overlay_index,
            (
                change.active,
                [current[0] - imported[0], current[1] - imported[1]],
            ),
        );
    }
    placements
}

/// Moves and shows or hides the background sprites as the scripts left
/// their tile blocks (see [`overlay_placements`]).
pub fn apply_overlay_changes(
    world: &mut World,
    scene: &FieldSceneDescription,
    changes: &BTreeMap<u16, tantalus_game::field::world::OverlayChange>,
    player: Option<[f32; 3]>,
) {
    let placements = overlay_placements(scene, changes, player);
    spawn_borders(world, scene, changes);
    let mut sprites = world.query::<(
        &FieldSpriteIdentity,
        &FieldSpriteBase,
        &mut Transform,
        &mut Visibility,
    )>();
    for (identity, base, mut transform, mut visibility) in sprites.iter_mut(world) {
        let (active, offset) = placements
            .get(&identity.overlay)
            .copied()
            .unwrap_or((None, [0.0, 0.0]));
        // Pixel offsets grow down; Bevy y grows up.
        transform.translation = base.translation + Vec3::new(offset[0], -offset[1], 0.0);
        let shown = active.unwrap_or(base.active);
        *visibility = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// `SetTileShadeColor` (`EBG_overlaySetShadeColor`): a tile block's material color becomes
/// `color / 128` per channel, so 128 leaves it as drawn and more brightens it. Blended
/// layers keep their own material and are not tinted.
pub fn apply_tile_shade(world: &mut World, shade: &BTreeMap<i32, [u8; 3]>) {
    if shade.is_empty() {
        return;
    }
    let mut sprites = world.query::<(&FieldSpriteIdentity, &mut Sprite)>();
    for (identity, mut sprite) in sprites.iter_mut(world) {
        if let Some([r, g, b]) = shade.get(&(identity.overlay as i32)) {
            let alpha = sprite.color.alpha();
            sprite.color = Color::linear_rgba(
                f32::from(*r) / 128.0,
                f32::from(*g) / 128.0,
                f32::from(*b) / 128.0,
                alpha,
            );
        }
    }
}

/// Marks the sprite that carries a tile block's border quads.
#[derive(Component)]
pub struct OverlayBorder;

/// `FieldMap.CreateBorder`: four opaque quads of the attach color around
/// the block, `vrpMinX` wide at the sides and `vrpMinY` tall above and
/// below, moving with it. They are children of the block's first sprite,
/// so they follow it, hide with it and go when the field does.
fn spawn_borders(
    world: &mut World,
    scene: &FieldSceneDescription,
    changes: &BTreeMap<u16, tantalus_game::field::world::OverlayChange>,
) {
    for (index, change) in changes {
        let Some(color) = change.border else {
            continue;
        };
        let Some(overlay) = scene
            .overlays
            .iter()
            .find(|overlay| overlay.overlay_index == u32::from(*index))
        else {
            continue;
        };
        let mut sprites = world.query::<(Entity, &FieldSpriteIdentity, Has<OverlayBorder>)>();
        let Some((parent, placed, bordered)) = sprites
            .iter(world)
            .filter(|(_, identity, _)| identity.overlay == overlay.overlay_index)
            .min_by_key(|(_, identity, _)| identity.sprite)
            .and_then(|(entity, identity, bordered)| {
                let sprite = overlay.sprites.get(identity.sprite as usize)?;
                Some((entity, (sprite.pixel_offset, sprite.pixel_size), bordered))
            })
        else {
            continue;
        };
        if bordered {
            continue;
        }
        let (sprite_offset, sprite_size) = placed;
        // The block's top-left in pixels, as the importer places sprites:
        // scene offset less half the PSX frame plus the block's offset.
        let left = f32::from(scene.scene_offset[0]) - crate::projection::HALF_FIELD[0]
            + f32::from(overlay.current_offset[0]);
        let top = f32::from(scene.scene_offset[1]) - crate::projection::HALF_FIELD[1]
            + f32::from(overlay.current_offset[1]);
        let [width, height] = overlay.pixel_size.map(f32::from);
        let side = f32::from(scene.camera.viewport_x[0].unsigned_abs());
        let cap = f32::from(scene.camera.viewport_y[0].unsigned_abs());
        // (x0, y0, x1, y1) in pixels, y down.
        let quads = [
            [left - side, top, left, top + height],
            [left + width, top, left + width + side, top + height],
            [left, top - cap, left + width, top],
            [left, top + height, left + width, top + height + cap],
        ];
        // The parent sprite's center in pixels, which child offsets are
        // measured from.
        let center = [
            sprite_offset[0] as f32 + sprite_size[0] as f32 / 2.0,
            sprite_offset[1] as f32 + sprite_size[1] as f32 / 2.0,
        ];
        let fill = Color::srgb_u8(color[0], color[1], color[2]);
        let children: Vec<Entity> = quads
            .iter()
            .filter(|quad| quad[2] > quad[0] && quad[3] > quad[1])
            .map(|quad| {
                world
                    .spawn((
                        Sprite::from_color(fill, Vec2::new(quad[2] - quad[0], quad[3] - quad[1])),
                        Transform::from_xyz(
                            (quad[0] + quad[2]) / 2.0 - center[0],
                            -((quad[1] + quad[3]) / 2.0 - center[1]),
                            0.0,
                        ),
                    ))
                    .id()
            })
            .collect();
        world
            .entity_mut(parent)
            .insert(OverlayBorder)
            .add_children(&children);
    }
}

/// A blend-mode 1-3 sprite as a quad with `PsxBlendMaterial`; `None` when
/// the plugin is missing or the atlas is not loaded.
fn spawn_blended(
    world: &mut World,
    sprite: &FieldSpritePresentation,
    image: Handle<Image>,
    base: FieldSpriteBase,
    identity: FieldSpriteIdentity,
) -> Option<Entity> {
    use crate::psx_blend::PsxBlendMaterial;
    let size = world
        .get_resource::<Assets<Image>>()?
        .get(&image)?
        .size_f32();
    let rect = sprite.atlas_rect.map(|value| value as f32);
    let material = PsxBlendMaterial::new(image, rect, [size.x, size.y], sprite.alpha)?;
    let material = world
        .get_resource_mut::<Assets<PsxBlendMaterial>>()?
        .add(material);
    let mesh = world
        .get_resource_mut::<Assets<Mesh>>()?
        .add(Rectangle::new(
            sprite.pixel_size[0] as f32,
            sprite.pixel_size[1] as f32,
        ));
    Some(
        world
            .spawn((
                Mesh2d(mesh),
                MeshMaterial2d(material),
                Transform::from_translation(base.translation),
                base.visibility(),
                base,
                identity,
            ))
            .id(),
    )
}

/// Spawns the background sprites and a 2D camera at the start of the field
/// camera's scroll range; returns every entity spawned.
pub fn spawn_field_scene(
    world: &mut World,
    scene: &FieldSceneDescription,
    sprites: &[&FieldSpritePresentation],
    assets: &ResolvedAssets,
    target: Option<RenderTarget>,
) -> Vec<Entity> {
    let mut entities = Vec::with_capacity(sprites.len() + 1);
    for sprite in sprites {
        let image = assets.images[&sprite.atlas].clone();
        let translation = Vec3::new(
            sprite.pixel_offset[0] as f32 + sprite.pixel_size[0] as f32 / 2.0,
            -(sprite.pixel_offset[1] as f32 + sprite.pixel_size[1] as f32 / 2.0),
            crate::projection::layer_z(sprite.depth as f32),
        );
        let identity = FieldSpriteIdentity {
            overlay: sprite.overlay_index,
            sprite: sprite.sprite_index,
        };
        let base = FieldSpriteBase {
            translation,
            active: sprite.active,
        };
        // Blend modes 1-3 (add, subtract, add) go through their own
        // blend state; without the plugin they are left out rather than
        // drawn opaque over the scene.
        if sprite.transparency != 0 && sprite.alpha != 0 {
            if let Some(entity) = spawn_blended(world, sprite, image, base, identity) {
                entities.push(entity);
            }
            continue;
        }
        entities.push(
            world
                .spawn((
                    Sprite {
                        image,
                        // Blend mode 0 (`abr_0`) is half background, half
                        // sprite.
                        color: if sprite.transparency != 0 {
                            Color::srgba(1.0, 1.0, 1.0, 0.5)
                        } else {
                            Color::WHITE
                        },
                        rect: Some(Rect::new(
                            sprite.atlas_rect[0] as f32,
                            sprite.atlas_rect[1] as f32,
                            sprite.atlas_rect[2] as f32,
                            sprite.atlas_rect[3] as f32,
                        )),
                        custom_size: Some(Vec2::new(
                            sprite.pixel_size[0] as f32,
                            sprite.pixel_size[1] as f32,
                        )),
                        ..Sprite::default()
                    },
                    Transform::from_translation(translation),
                    base.visibility(),
                    base,
                    identity,
                ))
                .id(),
        );
    }
    // Start of the scroll range, as a field shows before the camera
    // follows anyone.
    let range = crate::projection::view_range(&scene.camera);
    let transform = Transform::from_xyz(range[0], range[3], 500.0);
    let camera = match target {
        Some(target) => world.spawn((Camera2d, transform, target)).id(),
        None => world.spawn((Camera2d, transform)).id(),
    };
    entities.push(camera);
    entities
}
