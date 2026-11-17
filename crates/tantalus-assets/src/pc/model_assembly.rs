//! Render-ready model assembly (data side, no GPU code).
//!
//! A model directory in `p0data4.bin` (`assets/resources/models/<type>/<geo
//! id>/...`) holds a prefab: a root `GameObject` whose `Transform` tree is the
//! skeleton, `SkinnedMeshRenderer`s that point at `Mesh`, bone transforms and
//! `Material`s, and `Texture2D`s behind the materials. [`assemble_model`]
//! resolves all of that into plain Rust data ([`AssembledModel`]).
//!
//! [`pose`] binds legacy `AnimationClip`s (`p0data5.bin`) to the node tree by
//! curve path, samples clips to local TRS, and skins vertices on the CPU.
//!
//! Naming, verified against the real containers and the pinned Memoria
//! reference (`ModelFactory`, `AnimationFactory`, `FF9BattleDB.GEO`):
//!
//! * model directory `models/<type>/<geo id>/`; `<type>` is Memoria's
//!   `ModelType` (1 acc, 2 main, 3 mon, 4 npc, 5 sub);
//! * animation directory `animations/<geo id>/<animation id>.anim`; the geo
//!   id is the same number as the model directory name, and the animation id
//!   is a `FF9DBAll.AnimationDB` key.

use crate::install::ImportError;
use crate::pc::asset_bundle::AssetBundleMetadata;
use crate::pc::serialized::SerializedFile;
use crate::pc::texture::Texture2D;
use crate::pc::unity_object::mesh::BoneInfluence;
use crate::pc::unity_object::{
    Aabb, AnimationClip, GameObject, Material, Mesh, PPtr, SkinnedMeshRenderer, TypeTree,
    UnityObject, Value, decode_object,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

mod clip_sampling;
mod geo_names;
pub mod pose;

pub use clip_sampling::{
    ClipDuration, ClipDurationsReport, ClipSamplingReport, SAMPLING_TOLERANCE, clip_durations,
    clip_sampling_report,
};

use pose::{
    Interpolation, Mat4, NodePose, bind_clip_paths, bounds_of, rest_pose, sample_clip,
    sample_full_curve, skin_model, transform_point, union_bounds, world_matrices,
};

const MODELS_PREFIX: &str = "assets/resources/models/";
const ANIMATIONS_PREFIX: &str = "assets/resources/animations/";
const ISSUE_SAMPLES: usize = 12;

const CLASS_TRANSFORM: i32 = 4;
const CLASS_SKINNED_MESH_RENDERER: i32 = 137;
const CLASS_ANIMATION: i32 = 111;

/// Memoria `ModelType` directory number as a name.
pub fn model_type_name(model_type: u32) -> &'static str {
    match model_type {
        1 => "acc",
        2 => "main",
        3 => "mon",
        4 => "npc",
        5 => "sub",
        6 => "battle_weapon",
        _ => "unknown",
    }
}

/// Memoria geo name (`GEO_MAIN_F0_ZDN`) for a geo id, from `FF9BattleDB.GEO`.
pub fn geo_name(geo_id: u32) -> Option<&'static str> {
    geo_names::GEO_NAMES
        .binary_search_by_key(&geo_id, |(id, _)| *id)
        .ok()
        .map(|index| geo_names::GEO_NAMES[index].1)
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ModelNode {
    pub name: String,
    /// Path from the root's children (the root itself is `""`), joined by
    /// `/`: the form legacy animation curves use.
    pub path: String,
    /// Parent node index; always smaller than this node's index.
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub local_position: [f32; 3],
    /// Quaternion `[x, y, z, w]`.
    pub local_rotation: [f32; 4],
    pub local_scale: [f32; 3],
    /// Path id of the `Transform` object.
    pub transform_id: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterialTexture {
    pub property: String,
    pub texture_name: String,
    pub texture_id: i64,
    pub width: i32,
    pub height: i32,
    /// Unity `TextureFormat` number (12 is DXT5).
    pub format: i32,
    /// Top-row-first RGBA8, `width * height * 4` bytes, when decodable.
    pub rgba: Option<Vec<u8>>,
    /// Why `rgba` is absent.
    pub undecodable: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssembledMaterial {
    pub name: String,
    pub material_id: i64,
    pub textures: Vec<MaterialTexture>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartSubmesh {
    /// First index into [`SkinnedPart::indices`].
    pub first_index: u32,
    pub index_count: u32,
    /// Unity `MeshTopology` (0 = triangles).
    pub topology: i32,
    /// `SkinnedMeshRenderer.m_Materials[submesh]`, when present and resolved.
    pub material: Option<AssembledMaterial>,
}

/// One `SkinnedMeshRenderer` with its mesh data.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinnedPart {
    /// Name of the renderer's GameObject.
    pub name: String,
    /// Node of the renderer's GameObject.
    pub node: usize,
    pub mesh_name: String,
    pub mesh_id: i64,
    pub positions: Vec<[f32; 3]>,
    /// Empty when the mesh has no normal channel.
    pub normals: Vec<[f32; 3]>,
    /// First UV set; empty when absent.
    pub uvs: Vec<[f32; 2]>,
    /// Vertex colors (channel 2) as RGBA in 0..=1; empty when absent.
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub submeshes: Vec<PartSubmesh>,
    /// Per vertex; empty for an unskinned mesh.
    pub skin: Vec<BoneInfluence>,
    /// Node index for `m_Bones[i]`; `None` when the reference did not
    /// resolve to a node of this model.
    pub bones: Vec<Option<usize>>,
    /// `Mesh.m_BindPose[i]` (row-major, see [`pose`]).
    pub bind_poses: Vec<Mat4>,
    pub root_bone: Option<usize>,
    /// Mesh `m_LocalAABB`.
    pub mesh_bounds: Aabb,
    /// Renderer `m_AABB`.
    pub renderer_bounds: Aabb,
    /// Bounds of `positions` (mesh space).
    pub vertex_bounds: Option<Aabb>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssembledModel {
    /// Directory label, `<type>/<geo id>` for catalog-built models.
    pub label: String,
    /// Parent-before-child (preorder) node list; node 0 is the prefab root.
    pub nodes: Vec<ModelNode>,
    pub parts: Vec<SkinnedPart>,
    /// Path ids of `AnimationClip`s on the root `Animation` component.
    pub embedded_clips: Vec<i64>,
    /// Bind-pose bounds: every part's mesh vertices carried by its renderer
    /// node's prefab world matrix, in model space.
    pub bind_bounds: Option<Aabb>,
    /// Bounds of the parts skinned with the prefab node pose.
    pub prefab_pose_bounds: Option<Aabb>,
    /// Largest distance between CPU skinning with the prefab node pose and
    /// the bind-pose vertex position. Zero when the prefab pose is the bind
    /// pose; large for models whose prefab keeps an animated first frame.
    pub rest_skin_error: f32,
    pub issues: ModelIssues,
    /// Path ids of every object the assembly read.
    pub reached: BTreeSet<i64>,
    /// The model's texture animations (`Models/GeoTexAnim/<geo name>.tab`), set by
    /// [`geo_tex_anim::load`] after assembly.
    pub tex_anim: Option<crate::pc::geo_tex_anim::GeoTexAnim>,
}

/// Unresolved or inconsistent references found while assembling.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ModelIssues {
    pub bone_unresolved: usize,
    pub bone_count_mismatch: usize,
    pub mesh_unresolved: usize,
    pub material_unresolved: usize,
    pub texture_unresolved: usize,
    pub hierarchy_unresolved: usize,
    pub samples: Vec<String>,
}

impl ModelIssues {
    pub fn total(&self) -> usize {
        self.bone_unresolved
            + self.bone_count_mismatch
            + self.mesh_unresolved
            + self.material_unresolved
            + self.texture_unresolved
            + self.hierarchy_unresolved
    }

    fn note(&mut self, message: String) {
        if self.samples.len() < ISSUE_SAMPLES {
            self.samples.push(message);
        }
    }
}

impl AssembledModel {
    /// Node path to node index; the first node with a path wins, as in
    /// Unity's path resolution.
    pub fn path_lookup(&self) -> HashMap<&str, usize> {
        let mut lookup = HashMap::with_capacity(self.nodes.len());
        for (index, node) in self.nodes.iter().enumerate() {
            lookup.entry(node.path.as_str()).or_insert(index);
        }
        lookup
    }

    pub fn max_depth(&self) -> usize {
        let mut depth = vec![0_usize; self.nodes.len()];
        for (index, node) in self.nodes.iter().enumerate() {
            depth[index] = node.parent.map_or(0, |parent| depth[parent] + 1);
        }
        depth.into_iter().max().unwrap_or(0)
    }

    /// Releases decoded pixel data (summaries stay valid).
    pub fn drop_pixels(&mut self) {
        for part in &mut self.parts {
            for submesh in &mut part.submeshes {
                if let Some(material) = &mut submesh.material {
                    for texture in &mut material.textures {
                        texture.rgba = None;
                    }
                }
            }
        }
    }

    pub fn summary(&self) -> ModelSummary {
        let mut distinct_bones = BTreeSet::new();
        let mut bone_slots = 0;
        let mut materials = BTreeSet::new();
        let mut texture_ids = BTreeMap::new();
        let mut submeshes = 0;
        let mut submeshes_without_material = 0;
        for part in &self.parts {
            bone_slots += part.bones.len();
            distinct_bones.extend(part.bones.iter().flatten().copied());
            for submesh in &part.submeshes {
                submeshes += 1;
                match &submesh.material {
                    Some(material) => {
                        materials.insert(material.material_id);
                        for texture in &material.textures {
                            texture_ids.insert(texture.texture_id, texture);
                        }
                    }
                    None => submeshes_without_material += 1,
                }
            }
        }
        let decoded = texture_ids
            .values()
            .filter(|texture| texture.undecodable.is_none())
            .count();
        let texture_failures = texture_ids
            .values()
            .filter_map(|texture| {
                texture.undecodable.as_ref().map(|reason| {
                    format!("{} ({}): {reason}", texture.texture_name, texture.format)
                })
            })
            .collect();
        ModelSummary {
            label: self.label.clone(),
            nodes: self.nodes.len(),
            max_depth: self.max_depth(),
            parts: self.parts.len(),
            vertices: self.parts.iter().map(|p| p.positions.len()).sum(),
            indices: self.parts.iter().map(|p| p.indices.len()).sum(),
            submeshes,
            submeshes_without_material,
            distinct_bones: distinct_bones.len(),
            bone_slots,
            bone_resolution_failures: self.issues.bone_unresolved,
            materials: materials.len(),
            textures: texture_ids.len(),
            textures_decoded: decoded,
            textures_undecodable: texture_ids.len() - decoded,
            texture_failures,
            embedded_clips: self.embedded_clips.len(),
            bind_bounds: self.bind_bounds.map(BoundsOut::from),
            rest_skin_error: self.rest_skin_error,
            issues: self.issues.clone(),
            reached_objects: self.reached.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct BoundsOut {
    pub center: [f32; 3],
    pub extent: [f32; 3],
}

impl From<Aabb> for BoundsOut {
    fn from(value: Aabb) -> Self {
        Self {
            center: value.center,
            extent: value.extent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelSummary {
    pub label: String,
    pub nodes: usize,
    pub max_depth: usize,
    pub parts: usize,
    pub vertices: usize,
    pub indices: usize,
    pub submeshes: usize,
    pub submeshes_without_material: usize,
    /// Distinct skeleton nodes used as skinning bones.
    pub distinct_bones: usize,
    /// Sum of every part's `m_Bones` length.
    pub bone_slots: usize,
    pub bone_resolution_failures: usize,
    pub materials: usize,
    pub textures: usize,
    pub textures_decoded: usize,
    pub textures_undecodable: usize,
    pub texture_failures: Vec<String>,
    pub embedded_clips: usize,
    pub bind_bounds: Option<BoundsOut>,
    pub rest_skin_error: f32,
    pub issues: ModelIssues,
    pub reached_objects: usize,
}

// ---------------------------------------------------------------------------
// Object access
// ---------------------------------------------------------------------------

/// Where assembly reads objects from. `None` means no object has that path
/// id.
pub trait ObjectStore {
    fn load(&self, path_id: i64) -> Result<Option<UnityObject>, ImportError>;
    fn texture(&self, path_id: i64) -> Result<Option<Texture2D>, ImportError>;
}

/// [`ObjectStore`] over a parsed serialized file with embedded type trees.
pub struct SerializedStore<'f, 'a> {
    file: &'f SerializedFile<'a>,
    source: String,
    trees: HashMap<i32, TypeTree>,
    index: HashMap<i64, usize>,
}

impl<'f, 'a> SerializedStore<'f, 'a> {
    pub fn new(file: &'f SerializedFile<'a>, source: &str) -> Result<Self, ImportError> {
        let report = file.report();
        if !report.type_trees_embedded {
            return Err(ImportError::unsupported(
                source,
                "serialized file without embedded type trees",
            ));
        }
        let mut trees = HashMap::new();
        for object in &report.objects {
            if trees.contains_key(&object.type_id) {
                continue;
            }
            let raw = file.type_tree(object.type_id).ok_or_else(|| {
                ImportError::invalid(source, format!("no type tree for type {}", object.type_id))
            })?;
            trees.insert(object.type_id, TypeTree::parse(&raw, source)?);
        }
        let index = report
            .objects
            .iter()
            .enumerate()
            .map(|(index, object)| (object.path_id, index))
            .collect();
        Ok(Self {
            file,
            source: source.to_owned(),
            trees,
            index,
        })
    }

    pub fn class_of(&self, path_id: i64) -> Option<u16> {
        self.index
            .get(&path_id)
            .map(|index| self.file.report().objects[*index].class_id)
    }

    /// `(path id, class id)` of every object.
    pub fn objects(&self) -> impl Iterator<Item = (i64, u16)> + '_ {
        self.file
            .report()
            .objects
            .iter()
            .map(|object| (object.path_id, object.class_id))
    }

    /// Container entries of the `AssetBundle` object (class 142).
    pub fn container(&self) -> Result<Vec<ContainerEntry>, ImportError> {
        let report = self.file.report();
        let (index, _) = report
            .objects
            .iter()
            .enumerate()
            .find(|(_, object)| object.class_id == 142)
            .ok_or_else(|| ImportError::invalid(&self.source, "AssetBundle object is missing"))?;
        let metadata = AssetBundleMetadata::parse(self.file.object_data(index)?, &self.source)?;
        Ok(metadata
            .container
            .into_iter()
            .map(|(path, info)| ContainerEntry {
                class_id: self.class_of(info.path_id).unwrap_or(0),
                path,
                path_id: info.path_id,
            })
            .collect())
    }

    /// Reads the object at `path_id` through its type tree and hands the
    /// generic value to `read` (for classes without a typed view, such as
    /// `MeshFilter`). `None` when no object has that path id.
    pub fn read_value<T>(
        &self,
        path_id: i64,
        read: impl FnOnce(&Value<'_>, &str) -> Result<T, ImportError>,
    ) -> Result<Option<T>, ImportError> {
        let Some(index) = self.index.get(&path_id).copied() else {
            return Ok(None);
        };
        let object = &self.file.report().objects[index];
        let tree = self.trees.get(&object.type_id).ok_or_else(|| {
            ImportError::invalid(&self.source, format!("no type tree for {path_id}"))
        })?;
        let value = tree.read(self.file.object_data(index)?, &self.source)?;
        read(&value, &self.source).map(Some)
    }

    /// The bytes of the `TextAsset` at `path_id` (`None` when no object has
    /// that path id).
    pub fn text_asset(&self, path_id: i64) -> Result<Option<Vec<u8>>, ImportError> {
        let Some(index) = self.index.get(&path_id).copied() else {
            return Ok(None);
        };
        let bytes = self.file.object_data(index)?;
        Ok(Some(
            crate::pc::text_asset::TextAsset::parse(bytes, &self.source)?.script,
        ))
    }

    /// Decodes an `AnimationClip` by path id.
    pub fn clip(&self, path_id: i64) -> Result<Option<AnimationClip>, ImportError> {
        Ok(match self.load(path_id)? {
            Some(UnityObject::AnimationClip(clip)) => Some(*clip),
            _ => None,
        })
    }
}

impl ObjectStore for SerializedStore<'_, '_> {
    fn load(&self, path_id: i64) -> Result<Option<UnityObject>, ImportError> {
        let Some(index) = self.index.get(&path_id).copied() else {
            return Ok(None);
        };
        let object = &self.file.report().objects[index];
        let tree = self.trees.get(&object.type_id).ok_or_else(|| {
            ImportError::invalid(&self.source, format!("no type tree for {path_id}"))
        })?;
        decode_object(self.file, tree, index, &self.source).map(Some)
    }

    fn texture(&self, path_id: i64) -> Result<Option<Texture2D>, ImportError> {
        let Some(index) = self.index.get(&path_id).copied() else {
            return Ok(None);
        };
        if self.file.report().objects[index].class_id != 28 {
            return Ok(None);
        }
        Texture2D::parse(self.file.object_data(index)?, &self.source).map(Some)
    }
}

// ---------------------------------------------------------------------------
// Catalog: container paths to model directories
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerEntry {
    pub path: String,
    pub path_id: i64,
    pub class_id: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelDir {
    /// `<type>/<geo id>`.
    pub label: String,
    pub model_type: u32,
    pub type_name: &'static str,
    pub geo_id: u32,
    pub geo_name: Option<&'static str>,
    /// Container `GameObject` entries (the prefab root; normally one).
    pub roots: Vec<i64>,
    pub meshes: usize,
    pub textures: usize,
    pub materials: usize,
    pub clips: Vec<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelCatalog {
    pub dirs: BTreeMap<String, ModelDir>,
    /// Container entries under `models/` that are not in a numeric
    /// `<type>/<id>` directory (the `geotexanim` tables, for example).
    pub other_entries: Vec<String>,
}

impl ModelCatalog {
    pub fn from_container(entries: &[ContainerEntry]) -> Self {
        Self::from_container_with_prefix(entries, MODELS_PREFIX)
    }

    /// Same as [`Self::from_container`] for another `<type>/<geo id>/<file>` root: the battle
    /// bundle keeps weapon models under `assets/resources/battlemap/battlemodel/6/<geo id>/`.
    pub fn from_container_with_prefix(entries: &[ContainerEntry], prefix: &str) -> Self {
        let mut catalog = Self::default();
        for entry in entries {
            let Some(rest) = entry.path.strip_prefix(prefix) else {
                catalog.other_entries.push(entry.path.clone());
                continue;
            };
            let mut parts = rest.split('/');
            let (Some(kind), Some(id), Some(_)) = (parts.next(), parts.next(), parts.next()) else {
                catalog.other_entries.push(entry.path.clone());
                continue;
            };
            let (Ok(model_type), Ok(geo_id)) = (kind.parse::<u32>(), id.parse::<u32>()) else {
                catalog.other_entries.push(entry.path.clone());
                continue;
            };
            let dir = catalog
                .dirs
                .entry(format!("{model_type}/{geo_id}"))
                .or_insert_with(|| ModelDir {
                    label: format!("{model_type}/{geo_id}"),
                    model_type,
                    type_name: model_type_name(model_type),
                    geo_id,
                    geo_name: geo_name(geo_id),
                    roots: Vec::new(),
                    meshes: 0,
                    textures: 0,
                    materials: 0,
                    clips: Vec::new(),
                });
            match entry.class_id {
                1 => dir.roots.push(entry.path_id),
                43 => dir.meshes += 1,
                28 => dir.textures += 1,
                21 => dir.materials += 1,
                74 => dir.clips.push(entry.path_id),
                _ => {}
            }
        }
        catalog
    }

    /// Finds a directory by `<type>/<geo id>`, a geo id, or a geo name
    /// (case-insensitive). Ambiguous queries are an error.
    pub fn find(&self, query: &str) -> Result<&ModelDir, String> {
        if let Some(dir) = self.dirs.get(query) {
            return Ok(dir);
        }
        let matches: Vec<&ModelDir> = if let Ok(id) = query.parse::<u32>() {
            self.dirs.values().filter(|dir| dir.geo_id == id).collect()
        } else {
            self.dirs
                .values()
                .filter(|dir| dir.geo_name.is_some_and(|n| n.eq_ignore_ascii_case(query)))
                .collect()
        };
        match matches.as_slice() {
            [one] => Ok(one),
            [] => Err(format!("no model directory matches `{query}`")),
            many => Err(format!(
                "`{query}` matches {} directories: {}",
                many.len(),
                many.iter()
                    .map(|dir| dir.label.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// Directories that carry a geo id.
    pub fn dirs_for_geo(&self, geo_id: u32) -> Vec<&ModelDir> {
        self.dirs
            .values()
            .filter(|dir| dir.geo_id == geo_id)
            .collect()
    }
}

/// One `.anim` container entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipEntry {
    /// Folder number under `animations/` (the geo id of its model).
    pub geo_id: u32,
    /// File stem: a `FF9DBAll.AnimationDB` key when numeric.
    pub name: String,
    pub path_id: i64,
}

/// Parses `assets/resources/animations/<geo id>/<name>.anim` entries;
/// returns the entries and the paths that did not fit the pattern.
pub fn clip_entries(entries: &[ContainerEntry]) -> (Vec<ClipEntry>, Vec<String>) {
    let mut clips = Vec::new();
    let mut other = Vec::new();
    for entry in entries {
        let parsed = entry
            .path
            .strip_prefix(ANIMATIONS_PREFIX)
            .and_then(|rest| rest.split_once('/'))
            .and_then(|(folder, file)| {
                let geo_id = folder.parse::<u32>().ok()?;
                let name = file.strip_suffix(".anim")?;
                (!name.contains('/')).then(|| (geo_id, name.to_owned()))
            });
        match parsed {
            Some((geo_id, name)) if entry.class_id == 74 => clips.push(ClipEntry {
                geo_id,
                name,
                path_id: entry.path_id,
            }),
            _ => other.push(entry.path.clone()),
        }
    }
    (clips, other)
}

// ---------------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct AssembleOptions {
    /// Decode texture pixels; when false only dimensions and the decodable
    /// verdict are recorded.
    pub decode_textures: bool,
}

impl Default for AssembleOptions {
    fn default() -> Self {
        Self {
            decode_textures: true,
        }
    }
}

struct Assembler<'s> {
    store: &'s dyn ObjectStore,
    options: AssembleOptions,
    nodes: Vec<ModelNode>,
    transform_nodes: HashMap<i64, usize>,
    renderers: Vec<(usize, SkinnedMeshRenderer)>,
    embedded_clips: Vec<i64>,
    issues: ModelIssues,
    reached: BTreeSet<i64>,
    textures: HashMap<i64, MaterialTexture>,
}

/// Assembles the prefab rooted at `root_game_object`.
pub fn assemble_model(
    store: &dyn ObjectStore,
    label: &str,
    root_game_object: i64,
    options: AssembleOptions,
) -> Result<AssembledModel, ImportError> {
    let mut assembler = Assembler {
        store,
        options,
        nodes: Vec::new(),
        transform_nodes: HashMap::new(),
        renderers: Vec::new(),
        embedded_clips: Vec::new(),
        issues: ModelIssues::default(),
        reached: BTreeSet::new(),
        textures: HashMap::new(),
    };
    let root = match store.load(root_game_object)? {
        Some(UnityObject::GameObject(game_object)) => game_object,
        _ => {
            return Err(ImportError::invalid(
                label,
                format!("root {root_game_object} is not a GameObject"),
            ));
        }
    };
    assembler.reached.insert(root_game_object);
    let root_transform = transform_of(&root).ok_or_else(|| {
        ImportError::invalid(label, format!("root `{}` has no Transform", root.name))
    })?;
    assembler.walk(label, root_transform, root, None, 0)?;
    assembler.finish(label)
}

fn transform_of(game_object: &GameObject) -> Option<PPtr> {
    game_object
        .components
        .iter()
        .find(|(class, _)| *class == CLASS_TRANSFORM)
        .map(|(_, pointer)| *pointer)
}

impl Assembler<'_> {
    fn unresolved(&mut self, counter: fn(&mut ModelIssues) -> &mut usize, message: String) {
        *counter(&mut self.issues) += 1;
        self.issues.note(message);
    }

    fn walk(
        &mut self,
        label: &str,
        transform_ref: PPtr,
        game_object: GameObject,
        parent: Option<usize>,
        depth: usize,
    ) -> Result<(), ImportError> {
        if depth > 256 {
            return Err(ImportError::invalid(label, "node tree deeper than 256"));
        }
        if self.transform_nodes.contains_key(&transform_ref.path_id) {
            return Err(ImportError::invalid(
                label,
                format!("transform {} is reachable twice", transform_ref.path_id),
            ));
        }
        let transform = match self.store.load(transform_ref.path_id)? {
            Some(UnityObject::Transform(transform)) => transform,
            _ => {
                return Err(ImportError::invalid(
                    label,
                    format!("{} is not a Transform", transform_ref.path_id),
                ));
            }
        };
        self.reached.insert(transform_ref.path_id);
        let index = self.nodes.len();
        let path = match parent {
            None => String::new(),
            Some(parent) if self.nodes[parent].path.is_empty() => game_object.name.clone(),
            Some(parent) => format!("{}/{}", self.nodes[parent].path, game_object.name),
        };
        self.nodes.push(ModelNode {
            name: game_object.name.clone(),
            path,
            parent,
            children: Vec::new(),
            local_position: transform.local_position,
            local_rotation: transform.local_rotation,
            local_scale: transform.local_scale,
            transform_id: transform_ref.path_id,
        });
        self.transform_nodes.insert(transform_ref.path_id, index);
        if let Some(parent) = parent {
            self.nodes[parent].children.push(index);
        }
        for (class, component) in &game_object.components {
            match *class {
                CLASS_SKINNED_MESH_RENDERER => match self.store.load(component.path_id)? {
                    Some(UnityObject::SkinnedMeshRenderer(renderer)) => {
                        self.reached.insert(component.path_id);
                        self.renderers.push((index, renderer));
                    }
                    _ => self.unresolved(
                        |issues| &mut issues.hierarchy_unresolved,
                        format!(
                            "`{}` renderer {} is not a SkinnedMeshRenderer",
                            game_object.name, component.path_id
                        ),
                    ),
                },
                CLASS_ANIMATION if parent.is_none() => {
                    if let Some(UnityObject::Animation(animation)) =
                        self.store.load(component.path_id)?
                    {
                        self.reached.insert(component.path_id);
                        for clip in std::iter::once(&animation.clip).chain(&animation.clips) {
                            if clip.is_local() && !self.embedded_clips.contains(&clip.path_id) {
                                self.embedded_clips.push(clip.path_id);
                                self.reached.insert(clip.path_id);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for child in &transform.children {
            let child_transform = match self.store.load(child.path_id)? {
                Some(UnityObject::Transform(child_transform)) => child_transform,
                _ => {
                    self.unresolved(
                        |issues| &mut issues.hierarchy_unresolved,
                        format!(
                            "child transform {} of `{}`",
                            child.path_id, game_object.name
                        ),
                    );
                    continue;
                }
            };
            let child_object = match self.store.load(child_transform.game_object.path_id)? {
                Some(UnityObject::GameObject(child_object)) => child_object,
                _ => {
                    self.unresolved(
                        |issues| &mut issues.hierarchy_unresolved,
                        format!("GameObject of transform {}", child.path_id),
                    );
                    continue;
                }
            };
            self.reached.insert(child_transform.game_object.path_id);
            self.walk(label, *child, child_object, Some(index), depth + 1)?;
        }
        Ok(())
    }

    fn finish(mut self, label: &str) -> Result<AssembledModel, ImportError> {
        let renderers = std::mem::take(&mut self.renderers);
        let mut parts = Vec::with_capacity(renderers.len());
        for (node, renderer) in renderers {
            if let Some(part) = self.build_part(node, renderer)? {
                parts.push(part);
            }
        }
        let mut model = AssembledModel {
            label: label.to_owned(),
            nodes: self.nodes,
            parts,
            embedded_clips: self.embedded_clips,
            bind_bounds: None,
            prefab_pose_bounds: None,
            rest_skin_error: 0.0,
            issues: self.issues,
            reached: self.reached,
            tex_anim: None,
        };
        let poses = rest_pose(&model);
        let world = world_matrices(&model.nodes, &poses);
        let skinned = skin_model(&model, &poses);
        model.prefab_pose_bounds = skinned
            .iter()
            .filter_map(|points| bounds_of(points))
            .reduce(union_bounds);
        let mut worst = 0.0_f32;
        let mut bind_bounds: Option<Aabb> = None;
        for (part, posed) in model.parts.iter().zip(&skinned) {
            let owner = world[part.node];
            let bound: Vec<[f32; 3]> = part
                .positions
                .iter()
                .map(|position| transform_point(&owner, *position))
                .collect();
            if let Some(bounds) = bounds_of(&bound) {
                bind_bounds = Some(bind_bounds.map_or(bounds, |held| union_bounds(held, bounds)));
            }
            if part.skin.is_empty() {
                continue;
            }
            for (expected, posed) in bound.iter().zip(posed) {
                for axis in 0..3 {
                    worst = worst.max((expected[axis] - posed[axis]).abs());
                }
            }
        }
        model.bind_bounds = bind_bounds;
        model.rest_skin_error = worst;
        Ok(model)
    }

    fn build_part(
        &mut self,
        node: usize,
        renderer: SkinnedMeshRenderer,
    ) -> Result<Option<SkinnedPart>, ImportError> {
        let name = self.nodes[node].name.clone();
        let mesh = match self.store.load(renderer.mesh.path_id)? {
            Some(UnityObject::Mesh(mesh)) if renderer.mesh.is_local() => *mesh,
            _ => {
                self.unresolved(
                    |issues| &mut issues.mesh_unresolved,
                    format!("renderer `{name}` mesh {:?}", renderer.mesh),
                );
                return Ok(None);
            }
        };
        self.reached.insert(renderer.mesh.path_id);
        let mut bones = Vec::with_capacity(renderer.bones.len());
        for bone in &renderer.bones {
            let resolved = bone
                .is_local()
                .then(|| self.transform_nodes.get(&bone.path_id).copied())
                .flatten();
            if resolved.is_none() {
                self.unresolved(
                    |issues| &mut issues.bone_unresolved,
                    format!("renderer `{name}` bone {bone:?} is not a node of this model"),
                );
            }
            bones.push(resolved);
        }
        if bones.len() != mesh.bind_poses.len() {
            self.unresolved(
                |issues| &mut issues.bone_count_mismatch,
                format!(
                    "renderer `{name}` has {} bones for {} bind poses",
                    bones.len(),
                    mesh.bind_poses.len()
                ),
            );
        }
        let root_bone = renderer
            .root_bone
            .is_local()
            .then(|| {
                self.transform_nodes
                    .get(&renderer.root_bone.path_id)
                    .copied()
            })
            .flatten();

        let positions = vertex_channel(&mesh, 0, 3);
        let normals = vertex_channel(&mesh, 1, 3);
        let uvs = vertex_channel(&mesh, 3, 2);
        let colors = vertex_channel(&mesh, 2, 4);
        let triple = |flat: Vec<Vec<f32>>| -> Vec<[f32; 3]> {
            flat.into_iter().map(|v| [v[0], v[1], v[2]]).collect()
        };
        let positions = triple(positions);
        let vertex_bounds = bounds_of(&positions);

        let mut submeshes = Vec::with_capacity(mesh.submeshes.len());
        let mut materials: HashMap<i64, Option<AssembledMaterial>> = HashMap::new();
        for (number, submesh) in mesh.submeshes.iter().enumerate() {
            let pointer = renderer.materials.get(number).copied();
            let material = match pointer {
                Some(pointer) if pointer.is_local() => match materials.entry(pointer.path_id) {
                    std::collections::hash_map::Entry::Occupied(held) => held.get().clone(),
                    std::collections::hash_map::Entry::Vacant(slot) => slot
                        .insert(self.build_material(&name, pointer.path_id)?)
                        .clone(),
                },
                _ => {
                    self.unresolved(
                        |issues| &mut issues.material_unresolved,
                        format!("renderer `{name}` submesh {number} has no material"),
                    );
                    None
                }
            };
            submeshes.push(PartSubmesh {
                first_index: submesh.first_byte / 2,
                index_count: submesh.index_count,
                topology: submesh.topology,
                material,
            });
        }
        Ok(Some(SkinnedPart {
            name,
            node,
            mesh_name: mesh.name.clone(),
            mesh_id: renderer.mesh.path_id,
            positions,
            normals: triple(normals),
            uvs: uvs.into_iter().map(|v| [v[0], v[1]]).collect(),
            colors: colors
                .into_iter()
                .map(|v| [v[0], v[1], v[2], v[3]])
                .collect(),
            indices: mesh.indices.iter().map(|&i| u32::from(i)).collect(),
            submeshes,
            skin: mesh.skin.clone(),
            bones,
            bind_poses: mesh.bind_poses.clone(),
            root_bone,
            mesh_bounds: mesh.local_aabb,
            renderer_bounds: renderer.aabb,
            vertex_bounds,
        }))
    }

    fn build_material(
        &mut self,
        owner: &str,
        material_id: i64,
    ) -> Result<Option<AssembledMaterial>, ImportError> {
        let material: Material = match self.store.load(material_id)? {
            Some(UnityObject::Material(material)) => material,
            _ => {
                self.unresolved(
                    |issues| &mut issues.material_unresolved,
                    format!("renderer `{owner}` material {material_id} is not a Material"),
                );
                return Ok(None);
            }
        };
        self.reached.insert(material_id);
        let mut textures = Vec::new();
        for (property, pointer) in &material.textures {
            if pointer.is_null() {
                continue;
            }
            if !pointer.is_local() {
                self.unresolved(
                    |issues| &mut issues.texture_unresolved,
                    format!(
                        "material `{}` slot {property} points at another file ({pointer:?})",
                        material.name
                    ),
                );
                continue;
            }
            let cached = self.textures.get(&pointer.path_id).cloned();
            let mut texture = match cached {
                Some(texture) => texture,
                None => {
                    let built = self.build_texture(property, pointer.path_id)?;
                    self.textures.insert(pointer.path_id, built.clone());
                    built
                }
            };
            if texture.undecodable.as_deref() == Some("texture object is missing") {
                self.unresolved(
                    |issues| &mut issues.texture_unresolved,
                    format!(
                        "material `{}` slot {property} texture {} is missing",
                        material.name, pointer.path_id
                    ),
                );
            }
            self.reached.insert(pointer.path_id);
            texture.property = property.clone();
            textures.push(texture);
        }
        Ok(Some(AssembledMaterial {
            name: material.name,
            material_id,
            textures,
        }))
    }

    fn build_texture(
        &mut self,
        property: &str,
        texture_id: i64,
    ) -> Result<MaterialTexture, ImportError> {
        let absent = |reason: String| MaterialTexture {
            property: property.to_owned(),
            texture_name: String::new(),
            texture_id,
            width: 0,
            height: 0,
            format: 0,
            rgba: None,
            undecodable: Some(reason),
        };
        let texture = match self.store.texture(texture_id) {
            Ok(Some(texture)) => texture,
            Ok(None) => return Ok(absent("texture object is missing".to_owned())),
            Err(error) => return Ok(absent(error.to_string())),
        };
        let first_level = texture.first_level_bytes();
        let verdict = if !Texture2D::is_decodable_format(texture.format) {
            Some(format!(
                "Texture2D format {} has no decoder in this reader",
                texture.format
            ))
        } else if texture.width <= 0 || texture.height <= 0 {
            Some(format!("invalid size {}x{}", texture.width, texture.height))
        } else if texture.image_data.len() < first_level {
            Some(format!(
                "image data has {} bytes; first mip needs {first_level}",
                texture.image_data.len(),
            ))
        } else {
            None
        };
        let (rgba, undecodable) = match verdict {
            Some(reason) => (None, Some(reason)),
            None if self.options.decode_textures => match texture.rgba_bytes() {
                Ok(pixels) => (Some(pixels), None),
                Err(error) => (None, Some(error.to_string())),
            },
            None => (None, None),
        };
        Ok(MaterialTexture {
            property: property.to_owned(),
            texture_name: texture.name,
            texture_id,
            width: texture.width,
            height: texture.height,
            format: texture.format,
            rgba,
            undecodable,
        })
    }
}

/// Decodes a float vertex channel into `dimension`-sized rows (padded with
/// zeros when the stored dimension is smaller); empty when absent.
pub(crate) fn vertex_channel(mesh: &Mesh, slot: usize, dimension: usize) -> Vec<Vec<f32>> {
    let Some(flat) = mesh.channel_f32(slot) else {
        return Vec::new();
    };
    let stored = usize::from(mesh.channels[slot].dimension).max(1);
    flat.chunks_exact(stored)
        .map(|row| {
            let mut out = vec![0.0; dimension];
            for (target, value) in out.iter_mut().zip(row) {
                *target = *value;
            }
            out
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Whole-bundle report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AssembleAllReport {
    pub model_directories: usize,
    pub directories_without_root: usize,
    /// Labels and container entry counts of directories with no root.
    pub rootless_directories: Vec<String>,
    pub assembled: usize,
    pub failed: Vec<String>,
    pub nodes: usize,
    pub skinned_parts: usize,
    pub vertices: usize,
    pub distinct_bones: usize,
    pub bone_slots: usize,
    pub bone_resolution_failures: usize,
    pub bone_count_mismatches: usize,
    pub mesh_unresolved: usize,
    pub material_unresolved: usize,
    pub texture_unresolved: usize,
    pub hierarchy_unresolved: usize,
    pub materials: usize,
    pub textures: usize,
    pub textures_decoded: usize,
    pub textures_undecodable: usize,
    pub texture_failure_reasons: BTreeMap<String, usize>,
    pub max_rest_skin_error: f32,
    /// Models whose rest-pose skinning differs from the renderer-space mesh
    /// by more than 1e-2 units, worst first (label, error).
    pub rest_skin_outliers: Vec<(String, f32)>,
    pub model_types: BTreeMap<String, usize>,
    pub geo_ids_with_name: usize,
    /// Objects of the model classes (GameObject, Transform, renderer, Mesh,
    /// Material, Texture2D) that no assembled model reached, by class id.
    pub unreached_objects: BTreeMap<u16, usize>,
    /// Container entries outside numeric model directories.
    pub other_container_entries: usize,
    pub models: Vec<ModelSummary>,
}

/// Assembles every model directory of a models bundle, keeping the
/// assembled models (pixels dropped) for further checks.
pub fn assemble_all(
    store: &SerializedStore<'_, '_>,
    catalog: &ModelCatalog,
    options: AssembleOptions,
) -> (AssembleAllReport, Vec<(ModelDir, AssembledModel)>) {
    let mut report = AssembleAllReport {
        model_directories: catalog.dirs.len(),
        other_container_entries: catalog.other_entries.len(),
        ..AssembleAllReport::default()
    };
    let mut reached: BTreeSet<i64> = BTreeSet::new();
    let mut kept = Vec::new();
    let mut seen_textures = BTreeSet::new();
    let mut seen_materials = BTreeSet::new();
    for dir in catalog.dirs.values() {
        *report
            .model_types
            .entry(dir.type_name.to_owned())
            .or_insert(0) += 1;
        report.geo_ids_with_name += usize::from(dir.geo_name.is_some());
        let Some(root) = dir.roots.first().copied() else {
            report.directories_without_root += 1;
            report.rootless_directories.push(format!(
                "{} ({} meshes, {} textures, {} materials, {} clips)",
                dir.label,
                dir.meshes,
                dir.textures,
                dir.materials,
                dir.clips.len()
            ));
            continue;
        };
        if dir.roots.len() > 1 {
            report.failed.push(format!(
                "{}: {} root GameObjects; used the first",
                dir.label,
                dir.roots.len()
            ));
        }
        match assemble_model(store, &dir.label, root, options) {
            Ok(mut model) => {
                report.assembled += 1;
                let summary = model.summary();
                report.nodes += summary.nodes;
                report.skinned_parts += summary.parts;
                report.vertices += summary.vertices;
                report.distinct_bones += summary.distinct_bones;
                report.bone_slots += summary.bone_slots;
                report.bone_resolution_failures += model.issues.bone_unresolved;
                report.bone_count_mismatches += model.issues.bone_count_mismatch;
                report.mesh_unresolved += model.issues.mesh_unresolved;
                report.material_unresolved += model.issues.material_unresolved;
                report.texture_unresolved += model.issues.texture_unresolved;
                report.hierarchy_unresolved += model.issues.hierarchy_unresolved;
                report.max_rest_skin_error = report.max_rest_skin_error.max(model.rest_skin_error);
                if model.rest_skin_error > 1e-2 {
                    report
                        .rest_skin_outliers
                        .push((dir.label.clone(), model.rest_skin_error));
                }
                for part in &model.parts {
                    for submesh in &part.submeshes {
                        let Some(material) = &submesh.material else {
                            continue;
                        };
                        seen_materials.insert(material.material_id);
                        for texture in &material.textures {
                            if !seen_textures.insert(texture.texture_id) {
                                continue;
                            }
                            match &texture.undecodable {
                                None => report.textures_decoded += 1,
                                Some(reason) => {
                                    report.textures_undecodable += 1;
                                    *report
                                        .texture_failure_reasons
                                        .entry(format!("format {}: {reason}", texture.format))
                                        .or_insert(0) += 1;
                                }
                            }
                        }
                    }
                }
                reached.extend(model.reached.iter().copied());
                report.models.push(summary);
                model.drop_pixels();
                kept.push((dir.clone(), model));
            }
            Err(error) => report.failed.push(format!("{}: {error}", dir.label)),
        }
    }
    report
        .rest_skin_outliers
        .sort_by(|a, b| b.1.total_cmp(&a.1));
    report.materials = seen_materials.len();
    report.textures = seen_textures.len();
    for (path_id, class) in store.objects() {
        if matches!(class, 1 | 4 | 21 | 28 | 43 | 137) && !reached.contains(&path_id) {
            *report.unreached_objects.entry(class).or_insert(0) += 1;
        }
    }
    (report, kept)
}

// ---------------------------------------------------------------------------
// Clip report
// ---------------------------------------------------------------------------

/// Skinned-bounds tolerance for the clip check, as a fraction of the
/// bind-pose bounds diagonal.
pub const BOUNDS_TOLERANCE: f32 = 0.25;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ClipReport {
    pub clips: usize,
    pub decode_failed: usize,
    pub clips_with_transform_curves: usize,
    pub transform_curves: usize,
    pub float_curves: usize,
    /// Clips whose folder geo id has a model directory (the pairing used).
    pub clips_with_paired_model: usize,
    pub clips_without_paired_model: usize,
    pub paired_fully_matched: usize,
    pub paired_partially_matched: usize,
    pub paired_none_matched: usize,
    pub paired_curves_matched: usize,
    pub paired_curves_unmatched: usize,
    /// Clips whose transform curves all match at least one of the unique
    /// model skeletons (any pairing).
    pub any_skeleton_fully_matched: usize,
    pub unique_skeletons: usize,
    /// Clips without a paired model that match some skeleton.
    pub unpaired_matching_some_skeleton: usize,
    /// Fully matched clips where every sampled bounds comparison ran.
    pub bounds_checked: usize,
    pub bounds_within_tolerance: usize,
    pub bounds_tolerance: f32,
    pub bounds_max_deviation: f32,
    pub bounds_median_deviation: f32,
    pub bounds_worst: Vec<String>,
    /// Deviation histogram: share of checked clips per bin.
    pub bounds_deviation_bins: BTreeMap<String, usize>,
    /// Fully matched clips where some scale curve differs from the prefab
    /// node scale by 10x or more at t=0 (unit-conversion scale on the
    /// prefab root bone).
    pub clips_with_scale_unit_mismatch: usize,
    pub bounds_checked_without_scale_mismatch: usize,
    pub bounds_within_without_scale_mismatch: usize,
    /// Models with at least one scale-unit-mismatched clip.
    pub models_with_scale_unit_mismatch: usize,
    pub models_with_scale_unit_mismatch_labels: Vec<String>,
    pub unmatched_path_samples: Vec<String>,
    pub folder_geo_ids_without_model: usize,
    /// Folders without a model directory, by Memoria geo-name prefix
    /// (`(no geo name)` when the id is not in `FF9BattleDB.GEO`).
    pub unpaired_folder_prefixes: BTreeMap<String, usize>,
    pub models_with_clips: usize,
}

/// Binds and samples every clip of an animation bundle against the
/// assembled models, pairing clips with models by folder geo id.
pub fn clip_report(
    animations: &SerializedStore<'_, '_>,
    clips: &[ClipEntry],
    catalog: &ModelCatalog,
    models: &[(ModelDir, AssembledModel)],
    mode: Interpolation,
) -> ClipReport {
    let mut report = ClipReport {
        clips: clips.len(),
        bounds_tolerance: BOUNDS_TOLERANCE,
        ..ClipReport::default()
    };
    let by_label: HashMap<&str, usize> = models
        .iter()
        .enumerate()
        .map(|(index, (dir, _))| (dir.label.as_str(), index))
        .collect();
    // Unique skeleton path sets, for the any-pairing check.
    let mut skeleton_ids: HashMap<Vec<&str>, usize> = HashMap::new();
    let mut skeletons: Vec<HashMap<&str, usize>> = Vec::new();
    for (_, model) in models {
        let lookup = model.path_lookup();
        let mut key: Vec<&str> = lookup.keys().copied().collect();
        key.sort_unstable();
        if let std::collections::hash_map::Entry::Vacant(slot) = skeleton_ids.entry(key) {
            slot.insert(skeletons.len());
            skeletons.push(lookup);
        }
    }
    report.unique_skeletons = skeletons.len();
    let mut geo_without_model = BTreeSet::new();
    let mut models_with_clips = BTreeSet::new();
    let mut mismatched_models: BTreeSet<usize> = BTreeSet::new();
    let mut deviations: Vec<f32> = Vec::new();
    let mut worst: Vec<(f32, String)> = Vec::new();

    for entry in clips {
        let clip = match animations.clip(entry.path_id) {
            Ok(Some(clip)) => clip,
            _ => {
                report.decode_failed += 1;
                continue;
            }
        };
        let transform_curves =
            clip.rotation_curves.len() + clip.position_curves.len() + clip.scale_curves.len();
        report.transform_curves += transform_curves;
        report.float_curves += clip.float_curves.len();
        if transform_curves > 0 {
            report.clips_with_transform_curves += 1;
        }
        let name = format!("{}/{}", entry.geo_id, entry.name);

        // Any-skeleton check.
        let curve_paths: BTreeSet<&str> = clip
            .rotation_curves
            .iter()
            .map(|c| c.path.as_str())
            .chain(clip.position_curves.iter().map(|c| c.path.as_str()))
            .chain(clip.scale_curves.iter().map(|c| c.path.as_str()))
            .collect();
        let any_match = skeletons
            .iter()
            .any(|lookup| curve_paths.iter().all(|path| lookup.contains_key(path)));
        report.any_skeleton_fully_matched += usize::from(any_match);

        // Paired check: folder geo id == model directory geo id.
        let paired: Vec<usize> = catalog
            .dirs_for_geo(entry.geo_id)
            .into_iter()
            .filter_map(|dir| by_label.get(dir.label.as_str()).copied())
            .collect();
        let Some(&model_index) = paired.first() else {
            report.clips_without_paired_model += 1;
            geo_without_model.insert(entry.geo_id);
            report.unpaired_matching_some_skeleton += usize::from(any_match);
            continue;
        };
        report.clips_with_paired_model += 1;
        models_with_clips.insert(model_index);
        let model = &models[model_index].1;
        let binding = bind_clip_paths(&model.path_lookup(), &clip);
        report.paired_curves_matched += binding.matched;
        report.paired_curves_unmatched += binding.unmatched;
        if binding.unmatched == 0 {
            report.paired_fully_matched += 1;
        } else if binding.matched == 0 {
            report.paired_none_matched += 1;
        } else {
            report.paired_partially_matched += 1;
        }
        if binding.unmatched > 0 && report.unmatched_path_samples.len() < 12 {
            report.unmatched_path_samples.push(format!(
                "{name} vs {}: {}",
                models[model_index].0.label,
                binding.unmatched_paths.join(" | ")
            ));
        }
        if binding.unmatched != 0 || transform_curves == 0 {
            continue;
        }
        let Some(bind) = model.bind_bounds else {
            continue;
        };
        let poses = sample_clip(model, &clip, &binding, 0.0, mode);
        let scale_mismatch = clip
            .scale_curves
            .iter()
            .zip(&binding.scale)
            .any(|(curve, node)| {
                let (Some(node), Some(value)) = (node, sample_full_curve(curve, 0.0, mode)) else {
                    return false;
                };
                let rest = model.nodes[*node].local_scale;
                (0..3).any(|axis| {
                    rest[axis].abs() > 1e-9 && {
                        let ratio = value[axis] / rest[axis];
                        !(0.1..=10.0).contains(&ratio.abs())
                    }
                })
            });
        if scale_mismatch {
            report.clips_with_scale_unit_mismatch += 1;
            mismatched_models.insert(model_index);
        }
        let Some(posed) = pose::skinned_bounds(model, &poses) else {
            continue;
        };
        let deviation = bounds_deviation(&bind, &posed);
        report.bounds_checked += 1;
        report.bounds_within_tolerance += usize::from(deviation <= BOUNDS_TOLERANCE);
        if !scale_mismatch {
            report.bounds_checked_without_scale_mismatch += 1;
            report.bounds_within_without_scale_mismatch +=
                usize::from(deviation <= BOUNDS_TOLERANCE);
        }
        let bin = match deviation {
            d if d <= 0.1 => "<=0.10",
            d if d <= 0.25 => "<=0.25",
            d if d <= 0.5 => "<=0.50",
            d if d <= 1.0 => "<=1.00",
            _ => ">1.00",
        };
        *report
            .bounds_deviation_bins
            .entry(bin.to_owned())
            .or_insert(0) += 1;
        deviations.push(deviation);
        if !scale_mismatch {
            worst.push((
                deviation,
                format!("{name} vs {}: {deviation:.3}", models[model_index].0.label),
            ));
        }
    }
    report.folder_geo_ids_without_model = geo_without_model.len();
    for geo_id in &geo_without_model {
        let prefix = geo_name(*geo_id).map_or_else(
            || "(no geo name)".to_owned(),
            |name| name.splitn(3, '_').take(2).collect::<Vec<_>>().join("_"),
        );
        *report.unpaired_folder_prefixes.entry(prefix).or_insert(0) += 1;
    }
    report.models_with_clips = models_with_clips.len();
    report.models_with_scale_unit_mismatch = mismatched_models.len();
    report.models_with_scale_unit_mismatch_labels = mismatched_models
        .iter()
        .take(12)
        .map(|index| models[*index].0.label.clone())
        .collect();
    deviations.sort_by(f32::total_cmp);
    report.bounds_max_deviation = deviations.last().copied().unwrap_or(0.0);
    report.bounds_median_deviation = deviations.get(deviations.len() / 2).copied().unwrap_or(0.0);
    worst.sort_by(|a, b| b.0.total_cmp(&a.0));
    report.bounds_worst = worst.into_iter().take(8).map(|(_, text)| text).collect();
    report
}

/// Clips embedded in the models bundle, checked against their own prefab.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct EmbeddedClipReport {
    pub models_with_embedded_clips: usize,
    pub clips: usize,
    pub decode_failed: usize,
    pub fully_matched: usize,
    /// Fully matched clips whose t=0 local TRS equals the prefab pose of
    /// every node within [`POSE_TOLERANCE`].
    pub t0_equals_prefab: usize,
    pub worst_pose_error: f32,
    pub worst: Vec<String>,
}

/// Local TRS agreement tolerance for the embedded-clip check.
pub const POSE_TOLERANCE: f32 = 1e-3;

/// Largest difference between two local poses: position and scale as
/// absolute error scaled by `1 + |value|`, rotation as `1 - |dot|` of the
/// normalized quaternions.
pub fn pose_error(a: &NodePose, b: &NodePose) -> f32 {
    let mut worst = 0.0_f32;
    for axis in 0..3 {
        worst = worst
            .max((a.position[axis] - b.position[axis]).abs() / (1.0 + a.position[axis].abs()))
            .max((a.scale[axis] - b.scale[axis]).abs() / (1.0 + a.scale[axis].abs()));
    }
    let (qa, qb) = (
        pose::normalize_quat(a.rotation),
        pose::normalize_quat(b.rotation),
    );
    let dot: f32 = qa.iter().zip(&qb).map(|(x, y)| x * y).sum();
    worst.max(1.0 - dot.abs())
}

/// Binds each model's embedded clips (`Animation` component on the prefab
/// root) to that model and compares the t=0 pose with the prefab pose.
pub fn embedded_clip_report(
    store: &SerializedStore<'_, '_>,
    models: &[(ModelDir, AssembledModel)],
    mode: Interpolation,
) -> EmbeddedClipReport {
    let mut report = EmbeddedClipReport::default();
    for (dir, model) in models {
        if model.embedded_clips.is_empty() {
            continue;
        }
        report.models_with_embedded_clips += 1;
        let rest = rest_pose(model);
        for clip_id in &model.embedded_clips {
            report.clips += 1;
            let Ok(Some(clip)) = store.clip(*clip_id) else {
                report.decode_failed += 1;
                continue;
            };
            let binding = pose::bind_clip(model, &clip);
            if !binding.fully_matched() {
                continue;
            }
            report.fully_matched += 1;
            let posed = sample_clip(model, &clip, &binding, 0.0, mode);
            let error = posed
                .iter()
                .zip(&rest)
                .map(|(a, b)| pose_error(a, b))
                .fold(0.0_f32, f32::max);
            report.worst_pose_error = report.worst_pose_error.max(error);
            if error <= POSE_TOLERANCE {
                report.t0_equals_prefab += 1;
            } else if report.worst.len() < 8 {
                report
                    .worst
                    .push(format!("{} / {}: {error:.4}", dir.label, clip.name));
            }
        }
    }
    report
}

/// Largest min/max corner difference between two bounds, as a fraction of
/// the first bounds' diagonal.
pub fn bounds_deviation(bind: &Aabb, posed: &Aabb) -> f32 {
    let diagonal = (0..3)
        .map(|a| (bind.extent[a] * 2.0).powi(2))
        .sum::<f32>()
        .sqrt()
        .max(1e-6);
    let mut worst = 0.0_f32;
    for axis in 0..3 {
        let (bmin, bmax) = (
            bind.center[axis] - bind.extent[axis],
            bind.center[axis] + bind.extent[axis],
        );
        let (pmin, pmax) = (
            posed.center[axis] - posed.extent[axis],
            posed.center[axis] + posed.extent[axis],
        );
        worst = worst.max((bmin - pmin).abs()).max((bmax - pmax).abs());
    }
    worst / diagonal
}

// Re-exported so callers need only this module for the common flow.
pub use pose::{ClipBinding, bind_clip, sample_curve, skin_vertices, skinned_bounds};

#[cfg(test)]
mod tests;
