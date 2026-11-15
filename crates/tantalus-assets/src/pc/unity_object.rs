//! Unity 5.2 object decoding through embedded type trees.
//!
//! `p0data4.bin` (models) and `p0data5.bin` (animations) are Unity 5.2.3p2
//! serialized files with `enableTypeTree` set, so every object is decoded by
//! the generic [`type_tree`] reader and must consume its byte range exactly.
//! Typed views exist for Mesh, Transform, GameObject, SkinnedMeshRenderer,
//! Material, Animation and AnimationClip.

pub mod animation;
pub mod fields;
pub mod mesh;
pub mod scene;
pub mod type_tree;

pub use animation::AnimationClip;
pub use fields::{Aabb, PPtr};
pub use mesh::Mesh;
pub use scene::{AnimationComponent, GameObject, Material, SkinnedMeshRenderer, Transform};
pub use type_tree::{TypeTree, Value};

use crate::install::ImportError;
use crate::pc::serialized::SerializedFile;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

const FAILURE_SAMPLES: usize = 10;

/// Typed decode result for one object.
#[derive(Debug, Clone, PartialEq)]
pub enum UnityObject {
    GameObject(GameObject),
    Transform(Transform),
    Material(Material),
    Mesh(Box<Mesh>),
    AnimationClip(Box<AnimationClip>),
    Animation(AnimationComponent),
    SkinnedMeshRenderer(SkinnedMeshRenderer),
    /// Generic decode only (the class has no typed view).
    Other,
}

/// Decodes the object at `index` with its embedded type tree, then the typed
/// view for supported classes.
pub fn decode_object(
    file: &SerializedFile<'_>,
    tree: &TypeTree,
    index: usize,
    source: &str,
) -> Result<UnityObject, ImportError> {
    let object = &file.report().objects[index];
    let value = tree.read(file.object_data(index)?, source)?;
    decode_typed(object.class_id, &value, source)
}

pub fn decode_typed(
    class_id: u16,
    value: &Value<'_>,
    source: &str,
) -> Result<UnityObject, ImportError> {
    Ok(match class_id {
        scene::GAME_OBJECT_CLASS_ID => {
            UnityObject::GameObject(GameObject::from_value(value, source)?)
        }
        scene::TRANSFORM_CLASS_ID => UnityObject::Transform(Transform::from_value(value, source)?),
        scene::MATERIAL_CLASS_ID => UnityObject::Material(Material::from_value(value, source)?),
        mesh::MESH_CLASS_ID => UnityObject::Mesh(Box::new(Mesh::from_value(value, source)?)),
        animation::ANIMATION_CLIP_CLASS_ID => {
            UnityObject::AnimationClip(Box::new(AnimationClip::from_value(value, source)?))
        }
        scene::ANIMATION_CLASS_ID => {
            UnityObject::Animation(AnimationComponent::from_value(value, source)?)
        }
        scene::SKINNED_MESH_RENDERER_CLASS_ID => {
            UnityObject::SkinnedMeshRenderer(SkinnedMeshRenderer::from_value(value, source)?)
        }
        _ => UnityObject::Other,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectFailure {
    pub path_id: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassReport {
    pub class_id: u16,
    pub type_name: String,
    pub typed: bool,
    pub objects: usize,
    pub parsed: usize,
    pub failed: usize,
    pub bytes: u64,
    pub failure_samples: Vec<ObjectFailure>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MeshStats {
    pub meshes: usize,
    pub total_vertices: u64,
    pub max_vertices: u32,
    pub total_indices: u64,
    pub submeshes: usize,
    pub max_submeshes: usize,
    pub non_triangle_submeshes: usize,
    pub skinned_meshes: usize,
    pub total_bind_poses: u64,
    pub max_bind_poses: usize,
    pub blend_shape_meshes: usize,
    pub compressed_meshes: usize,
    pub multi_stream_meshes: usize,
    /// Meshes whose vertex blob ends with the 16-byte stream padding.
    pub vertex_blobs_with_tail_padding: usize,
    pub layouts: BTreeMap<String, usize>,
    pub positions_outside_local_aabb: usize,
    pub influences_with_bad_weight_sum: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnimationStats {
    pub clips: usize,
    pub legacy_clips: usize,
    pub compressed_clips: usize,
    pub clips_without_keys: usize,
    pub rotation_curves: u64,
    pub position_curves: u64,
    pub scale_curves: u64,
    pub float_curves: u64,
    pub compressed_rotation_curves: u64,
    pub pptr_curves: u64,
    pub total_keys: u64,
    pub max_curves: usize,
    pub max_length_seconds: f32,
    pub max_frames: u32,
    pub sample_rates: BTreeMap<String, usize>,
    pub muscle_clips: usize,
    pub events: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HierarchyStats {
    pub game_objects: usize,
    pub transforms: usize,
    pub root_transforms: usize,
    pub max_depth: usize,
    pub max_children: usize,
    pub broken_transform_links: usize,
    pub broken_component_links: usize,
    pub skinned_renderers: usize,
    pub renderer_mesh_unresolved: usize,
    pub renderer_bone_unresolved: usize,
    pub renderer_bone_count_mismatch: usize,
    pub materials: usize,
    pub material_texture_slots: usize,
    pub animation_components: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnityObjectsReport {
    pub unity_version: String,
    pub type_trees_embedded: bool,
    pub objects_selected: usize,
    pub parsed: usize,
    pub failed: usize,
    pub classes: Vec<ClassReport>,
    pub mesh: Option<MeshStats>,
    pub animation: Option<AnimationStats>,
    pub hierarchy: Option<HierarchyStats>,
}

/// Decodes every object (or every object of `class_filter`) and reports
/// parsed and failed counts plus per-class statistics and link checks.
pub fn inspect_unity_objects(
    file: &SerializedFile<'_>,
    class_filter: Option<u16>,
    source: &str,
) -> Result<UnityObjectsReport, ImportError> {
    let report = file.report();
    if !report.type_trees_embedded {
        return Err(ImportError::unsupported(
            source,
            "serialized file without embedded type trees",
        ));
    }
    let class_of: HashMap<i64, u16> = report
        .objects
        .iter()
        .map(|object| (object.path_id, object.class_id))
        .collect();
    let mut trees: HashMap<i32, Result<TypeTree, String>> = HashMap::new();
    let mut classes: BTreeMap<u16, ClassReport> = BTreeMap::new();
    let mut mesh_stats = MeshStats::default();
    let mut animation_stats = AnimationStats::default();
    let mut hierarchy = HierarchyStats::default();
    let mut transforms: HashMap<i64, Transform> = HashMap::new();
    let mut game_objects: Vec<GameObject> = Vec::new();
    let mut renderers: Vec<SkinnedMeshRenderer> = Vec::new();
    let mut mesh_bind_poses: HashMap<i64, usize> = HashMap::new();

    for (index, object) in report.objects.iter().enumerate() {
        if class_filter.is_some_and(|class| class != object.class_id) {
            continue;
        }
        let tree = trees.entry(object.type_id).or_insert_with(|| {
            file.type_tree(object.type_id)
                .ok_or_else(|| format!("no type tree for type {}", object.type_id))
                .and_then(|raw| TypeTree::parse(&raw, source).map_err(|error| error.to_string()))
        });
        let entry = classes
            .entry(object.class_id)
            .or_insert_with(|| ClassReport {
                class_id: object.class_id,
                type_name: tree
                    .as_ref()
                    .map(|tree| tree.root_type().to_owned())
                    .unwrap_or_default(),
                typed: matches!(object.class_id, 1 | 4 | 21 | 43 | 74 | 111 | 137),
                objects: 0,
                parsed: 0,
                failed: 0,
                bytes: 0,
                failure_samples: Vec::new(),
            });
        entry.objects += 1;
        entry.bytes += u64::from(object.size);
        let decoded = match tree {
            Ok(tree) => decode_object(file, tree, index, source).map_err(|error| error.to_string()),
            Err(message) => Err(message.clone()),
        };
        let decoded = match decoded {
            Ok(decoded) => decoded,
            Err(message) => {
                entry.failed += 1;
                if entry.failure_samples.len() < FAILURE_SAMPLES {
                    entry.failure_samples.push(ObjectFailure {
                        path_id: object.path_id,
                        message,
                    });
                }
                continue;
            }
        };
        entry.parsed += 1;
        match decoded {
            UnityObject::Mesh(mesh) => {
                record_mesh(&mut mesh_stats, &mesh);
                mesh_bind_poses.insert(object.path_id, mesh.bind_poses.len());
            }
            UnityObject::AnimationClip(clip) => record_clip(&mut animation_stats, &clip),
            UnityObject::Transform(transform) => {
                transforms.insert(object.path_id, transform);
            }
            UnityObject::GameObject(game_object) => game_objects.push(game_object),
            UnityObject::SkinnedMeshRenderer(renderer) => renderers.push(renderer),
            UnityObject::Material(material) => {
                hierarchy.materials += 1;
                hierarchy.material_texture_slots += material.textures.len();
            }
            UnityObject::Animation(_) => hierarchy.animation_components += 1,
            UnityObject::Other => {}
        }
    }

    let resolves = |pointer: &PPtr, class: u16| {
        pointer.file_id == 0 && class_of.get(&pointer.path_id) == Some(&class)
    };
    hierarchy.game_objects = game_objects.len();
    for game_object in &game_objects {
        for (class_id, component) in &game_object.components {
            let ok = component.file_id == 0
                && class_of
                    .get(&component.path_id)
                    .is_some_and(|class| i32::from(*class) == *class_id);
            if !ok {
                hierarchy.broken_component_links += 1;
            }
        }
    }
    hierarchy.transforms = transforms.len();
    let all_transforms = class_filter.is_none_or(|class| class == scene::TRANSFORM_CLASS_ID);
    for (path_id, transform) in &transforms {
        hierarchy.max_children = hierarchy.max_children.max(transform.children.len());
        if transform.father.is_null() {
            hierarchy.root_transforms += 1;
        } else if !resolves(&transform.father, scene::TRANSFORM_CLASS_ID) {
            hierarchy.broken_transform_links += 1;
        }
        if !resolves(&transform.game_object, scene::GAME_OBJECT_CLASS_ID) {
            hierarchy.broken_transform_links += 1;
        }
        for child in &transform.children {
            let back = transforms
                .get(&child.path_id)
                .is_some_and(|child| child.father.file_id == 0 && child.father.path_id == *path_id);
            if child.file_id != 0 || (all_transforms && !back) {
                hierarchy.broken_transform_links += 1;
            }
        }
        let mut depth = 0;
        let mut cursor = transform.father;
        while !cursor.is_null() && depth <= transforms.len() {
            depth += 1;
            cursor = match transforms.get(&cursor.path_id) {
                Some(parent) => parent.father,
                None => break,
            };
        }
        hierarchy.max_depth = hierarchy.max_depth.max(depth);
    }
    hierarchy.skinned_renderers = renderers.len();
    for renderer in &renderers {
        if !resolves(&renderer.mesh, mesh::MESH_CLASS_ID) {
            hierarchy.renderer_mesh_unresolved += 1;
        }
        if renderer
            .bones
            .iter()
            .any(|bone| !resolves(bone, scene::TRANSFORM_CLASS_ID))
        {
            hierarchy.renderer_bone_unresolved += 1;
        }
        if mesh_bind_poses
            .get(&renderer.mesh.path_id)
            .is_some_and(|poses| *poses != renderer.bones.len())
        {
            hierarchy.renderer_bone_count_mismatch += 1;
        }
    }

    let classes: Vec<ClassReport> = classes.into_values().collect();
    let parsed = classes.iter().map(|class| class.parsed).sum();
    let failed = classes.iter().map(|class| class.failed).sum();
    let has = |class: u16| classes.iter().any(|report| report.class_id == class);
    Ok(UnityObjectsReport {
        unity_version: report.unity_version.clone(),
        type_trees_embedded: report.type_trees_embedded,
        objects_selected: parsed + failed,
        parsed,
        failed,
        mesh: has(mesh::MESH_CLASS_ID).then_some(mesh_stats),
        animation: has(animation::ANIMATION_CLIP_CLASS_ID).then_some(animation_stats),
        hierarchy: [1, 4, 21, 111, 137]
            .into_iter()
            .any(has)
            .then_some(hierarchy),
        classes,
    })
}

fn record_mesh(stats: &mut MeshStats, mesh: &Mesh) {
    stats.meshes += 1;
    stats.total_vertices += u64::from(mesh.vertex_count);
    stats.max_vertices = stats.max_vertices.max(mesh.vertex_count);
    stats.total_indices += mesh.indices.len() as u64;
    stats.submeshes += mesh.submeshes.len();
    stats.max_submeshes = stats.max_submeshes.max(mesh.submeshes.len());
    stats.non_triangle_submeshes += mesh
        .submeshes
        .iter()
        .filter(|submesh| submesh.topology != 0)
        .count();
    if !mesh.skin.is_empty() {
        stats.skinned_meshes += 1;
    }
    stats.total_bind_poses += mesh.bind_poses.len() as u64;
    stats.max_bind_poses = stats.max_bind_poses.max(mesh.bind_poses.len());
    if mesh.blend_shape_channels > 0 {
        stats.blend_shape_meshes += 1;
    }
    if mesh.mesh_compression != 0 || mesh.compressed_vertex_items > 0 {
        stats.compressed_meshes += 1;
    }
    if mesh.streams.len() > 1 {
        stats.multi_stream_meshes += 1;
    }
    if mesh.streams.last().is_some_and(|last| {
        last.offset + mesh.vertex_count as usize * last.stride != mesh.vertex_data.len()
    }) {
        stats.vertex_blobs_with_tail_padding += 1;
    }
    *stats.layouts.entry(mesh.layout_signature()).or_insert(0) += 1;
    if let Some(positions) = mesh.channel_f32(0) {
        let dimension = usize::from(mesh.channels[0].dimension).max(1);
        let aabb = mesh.local_aabb;
        let outside = positions.chunks_exact(dimension).any(|vertex| {
            vertex.iter().take(3).enumerate().any(|(axis, value)| {
                let slack = aabb.extent[axis].abs() * 1e-3 + 1e-3;
                (value - aabb.center[axis]).abs() > aabb.extent[axis] + slack
            })
        });
        if outside {
            stats.positions_outside_local_aabb += 1;
        }
    }
    stats.influences_with_bad_weight_sum += mesh
        .skin
        .iter()
        .filter(|influence| (influence.weights.iter().sum::<f32>() - 1.0).abs() > 1e-3)
        .count() as u64;
}

fn record_clip(stats: &mut AnimationStats, clip: &AnimationClip) {
    stats.clips += 1;
    stats.legacy_clips += usize::from(clip.legacy);
    stats.compressed_clips += usize::from(clip.compressed);
    if clip.key_count() == 0 {
        stats.clips_without_keys += 1;
    }
    stats.rotation_curves += clip.rotation_curves.len() as u64;
    stats.position_curves += clip.position_curves.len() as u64;
    stats.scale_curves += clip.scale_curves.len() as u64;
    stats.float_curves += clip.float_curves.len() as u64;
    stats.compressed_rotation_curves += clip.compressed_rotation_curves as u64;
    stats.pptr_curves += clip.pptr_curves as u64;
    stats.total_keys += clip.key_count() as u64;
    stats.max_curves = stats.max_curves.max(clip.curve_count());
    stats.max_length_seconds = stats.max_length_seconds.max(clip.length());
    stats.max_frames = stats.max_frames.max(clip.frame_count());
    *stats
        .sample_rates
        .entry(format!("{}", clip.sample_rate))
        .or_insert(0) += 1;
    if clip.muscle_clip_size > 0 {
        stats.muscle_clips += 1;
    }
    stats.events += clip.events as u64;
}
