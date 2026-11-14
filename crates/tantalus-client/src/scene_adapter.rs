use crate::animation_adapter::AnimationAdapter;
use crate::legacy_compositor::LegacyCompositor;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{Display, Formatter};
use tantalus_contracts::{
    ContentId, ContentRevision, EntityId, Point3, PresentationSnapshot, ScenePresentationMode,
};

#[derive(Debug, Clone, Default)]
pub struct ResolvedAssets {
    pub content_revision: ContentRevision,
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
pub struct LayerIdentity(pub ContentId);

#[derive(Component)]
pub struct ActorIdentity(pub EntityId);
