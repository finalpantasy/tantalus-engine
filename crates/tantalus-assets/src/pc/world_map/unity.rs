//! Unity side of `p0data3.bin`: classification of every container entry, block
//! meshes, block prefabs (`WMBlockPrefab`) and the grid mapping between them.

use super::{MAP_GRID_HEIGHT, MAP_GRID_WIDTH, WmapAssetReport, analyze_wmap_asset};
use crate::install::ImportError;
use crate::pc::asset_bundle::AssetBundleMetadata;
use crate::pc::serialized::SerializedFile;
use crate::pc::text_asset::TextAsset;
use crate::pc::unity_object::fields::Fields;
use crate::pc::unity_object::type_tree::{TypeTree, Value};
use crate::pc::unity_object::{GameObject, PPtr, Transform, UnityObject, decode_object};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

const PREFIX: &str = "assets/resources/worldmap/";
const SAMPLE_LIMIT: usize = 20;
const WM_BLOCK_PREFAB_CLASS: &str = "WMBlockPrefab";
const GAME_OBJECT: u16 = 1;
const MATERIAL: u16 = 21;
const TEXTURE_2D: u16 = 28;
const MESH_FILTER: u16 = 33;
const MESH: u16 = 43;
const TEXT_ASSET: u16 = 49;
const MONO_BEHAVIOUR: u16 = 114;
const MONO_SCRIPT: u16 = 115;
const RENDER_TEXTURE: u16 = 84;

/// `Block[x][y] kind` parsed from a mesh or prefab name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockName {
    pub x: u32,
    pub y: u32,
    /// Lower-case suffix after the grid (`terrain`, `object`, `sea4`, ...); empty for
    /// prefabs.
    pub kind: String,
}

/// Parses `block[x][y]` optionally followed by ` kind`. Input is the lower-case
/// file stem.
pub fn parse_block_name(stem: &str) -> Option<BlockName> {
    let rest = stem.strip_prefix("block[")?;
    let (x, rest) = rest.split_once("][")?;
    let (y, rest) = rest.split_once(']')?;
    let kind = match rest {
        "" => String::new(),
        other => match other.strip_prefix(' ') {
            Some(kind) => kind.to_owned(),
            // `Block[12][0]f`: variant suffix with no space (the shared sea prefab).
            None if other.len() <= 2 && other.bytes().all(|b| b.is_ascii_lowercase()) => {
                other.to_owned()
            }
            None => return None,
        },
    };
    Some(BlockName {
        x: x.parse().ok()?,
        y: y.parse().ok()?,
        kind,
    })
}

/// `(disc, row folder, form, name)` of a block mesh or prefab entry.
pub type BlockDetail = (u32, u32, u32, BlockName);

/// Container-path classification: `(group, detail)`. `None` = unclassified.
pub fn classify_entry(path: &str, class_id: u16) -> Option<(String, Option<BlockDetail>)> {
    let rest = path.strip_prefix(PREFIX)?;
    let parts: Vec<&str> = rest.split('/').collect();
    let file = *parts.last()?;
    let extension = file.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
    let top = parts[0];
    let group = |name: &str| Some((name.to_owned(), None));
    match (top, extension, class_id) {
        ("disc1" | "disc4", "asset", MESH) if parts.len() == 4 => {
            // disc{N}/{folder}/r{Y}/block[x][y] kind.asset
            let disc: u32 = top.strip_prefix("disc")?.parse().ok()?;
            let row: u32 = parts[2].strip_prefix('r')?.parse().ok()?;
            let name = parse_block_name(stem)?;
            let folder: u32 = parts[1].strip_prefix("0_")?.parse().ok()?;
            Some((format!("{top}/block_mesh"), Some((disc, row, folder, name))))
        }
        ("prefabs", "prefab", GAME_OBJECT)
            if parts.len() == 4 && parts[1].starts_with("worlddisc") =>
        {
            let disc: u32 = parts[1].strip_prefix("worlddisc")?.parse().ok()?;
            let row: u32 = parts[2].strip_prefix('r')?.parse().ok()?;
            let name = parse_block_name(stem)?;
            Some((
                format!("prefabs/disc{disc}/block_prefab"),
                Some((disc, row, 1, name)),
            ))
        }
        ("prefabs", "prefab", GAME_OBJECT) if parts.get(1) == Some(&"effects") => {
            group("prefabs/effect_prefab")
        }
        ("prefabs", "prefab", GAME_OBJECT) if parts.get(1) == Some(&"extra") => {
            group("prefabs/extra_prefab")
        }
        ("prefabs", "prefab", GAME_OBJECT) if parts.len() == 2 => {
            group("prefabs/unused_root_prefab")
        }
        ("prefabs", "obj", GAME_OBJECT) => group("prefabs/embedded_object_prefab"),
        ("prefabs", "obj", MESH) => group("prefabs/embedded_object_mesh"),
        ("prefabs", "mat", MATERIAL) => group("prefabs/material"),
        ("prefabs", "png", TEXTURE_2D) => group("prefabs/texture"),
        ("effectmodels", "fbx", GAME_OBJECT) => group("effectmodels/model_object"),
        ("effectmodels", "fbx", MESH) => group("effectmodels/model_mesh"),
        ("effectmodels", "fbx", 74) => group("effectmodels/model_animation"),
        ("effectmodels", "asset", MESH) => group("effectmodels/mesh_asset"),
        ("effectmodels", "mat", MATERIAL) => group("effectmodels/material"),
        ("effectmodels", "png", TEXTURE_2D) => group("effectmodels/texture"),
        ("materials", "mat", MATERIAL) => group("materials/material"),
        ("materials", "png", TEXTURE_2D) => group("materials/texture"),
        ("materials_old", "mat", MATERIAL) => group("materials_old/material"),
        ("materials_old", "png", TEXTURE_2D) => group("materials_old/texture"),
        ("textures", "png", TEXTURE_2D) => group("textures/texture"),
        ("rendertextures", "rendertexture", RENDER_TEXTURE) => {
            group("rendertextures/render_texture")
        }
        ("rendertextures_old", "rendertexture", RENDER_TEXTURE) => {
            group("rendertextures_old/render_texture")
        }
        ("wmap", "bytes", TEXT_ASSET) => group("wmap/binary_text_asset"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupCount {
    pub group: String,
    pub class_id: u16,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BlockMeshReport {
    /// Walk/render form from the `0_<form>` folder (1 = form 1, 2 = form 2).
    pub form: u32,
    pub kind: String,
    pub path: String,
    pub vertices: u32,
    pub triangles: u32,
    pub submeshes: usize,
    pub non_triangle_submeshes: usize,
    pub has_tangent_channel: bool,
    /// Distinct walk-mesh ids (`tangent.x` of each triangle's first vertex, as in
    /// `WMBlock.Raycast`).
    pub walk_ids: Vec<u32>,
    pub mesh_name_matches_path: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrefabSlot {
    pub slot: String,
    pub object_name: String,
    pub mesh_path: Option<String>,
    /// The mesh's container path names the same disc and grid cell as the prefab.
    pub mesh_matches_block: bool,
    /// Sea blocks (`IsSea`) share the meshes of disc 1 `Block[12][0]` (`SeaBlockPrefab`).
    pub shared_sea_mesh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrefabReport {
    pub prefab_path: String,
    pub variant: String,
    pub slots: Vec<PrefabSlot>,
    /// `WMBlockPrefab` Boolean/Int32 fields (`IsSea`, `InitialX`, ...).
    pub fields: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BlockCellReport {
    pub disc: u32,
    pub x: u32,
    pub y: u32,
    pub meshes: Vec<BlockMeshReport>,
    pub prefab: Option<PrefabReport>,
    /// Prefabs with a suffix after the grid (`Block[12][0]f`, the shared sea prefab).
    pub variant_prefabs: Vec<PrefabReport>,
    pub vertices: u64,
    pub triangles: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiscSummary {
    pub disc: u32,
    pub cells_with_meshes: usize,
    pub block_meshes: usize,
    pub block_prefabs: usize,
    pub meshes_by_kind: BTreeMap<String, usize>,
    pub vertices: u64,
    pub triangles: u64,
    pub grid_min_x: u32,
    pub grid_max_x: u32,
    pub grid_min_y: u32,
    pub grid_max_y: u32,
    pub prefab_slots: usize,
    pub prefab_slots_matching_block: usize,
    pub prefab_slots_shared_sea_mesh: usize,
    pub unreferenced_mesh_kinds: BTreeMap<String, usize>,
    pub prefabs_without_meshes: usize,
    pub meshes_not_in_any_prefab: usize,
    pub meshes_in_more_than_one_prefab: usize,
    pub prefab_flag_totals: BTreeMap<String, i64>,
    pub prefab_initial_xy_mismatch: usize,
    /// Prefab `IsSea` versus `map.bin`: `(map value, cells)` for sea cells.
    pub map_values_for_sea_cells: BTreeMap<u8, usize>,
    pub map_values_for_land_cells: BTreeMap<u8, usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ObjectSummary {
    pub materials: usize,
    pub materials_decoded: usize,
    pub material_texture_slots: usize,
    pub textures: usize,
    pub textures_decoded: usize,
    pub texture_formats: BTreeMap<String, usize>,
    pub texture_max_dimension: u32,
    pub render_textures: usize,
    pub other_meshes: usize,
    pub other_meshes_decoded: usize,
    pub game_objects: usize,
    pub game_objects_decoded: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorldMapReport {
    pub bundle_object_count: usize,
    pub class_counts: BTreeMap<u16, usize>,
    pub container_entries: usize,
    /// Distinct container paths (an FBX path maps to several sub-assets).
    pub distinct_container_paths: usize,
    pub classified_entries: usize,
    pub unclassified_entries: usize,
    pub unclassified_samples: Vec<String>,
    pub groups: Vec<GroupCount>,
    pub block_mesh_folders: BTreeMap<String, usize>,
    pub block_meshes_decoded: usize,
    pub block_meshes_failed: usize,
    pub block_mesh_vertices: u64,
    pub block_mesh_triangles: u64,
    pub block_mesh_name_path_mismatches: usize,
    pub block_mesh_row_mismatches: usize,
    pub discs: Vec<DiscSummary>,
    pub objects: ObjectSummary,
    pub wmap_assets: Vec<WmapAssetReport>,
    pub wmap_bytes_total: usize,
    pub wmap_bytes_parsed: usize,
    pub wmap_bytes_zero_padding: usize,
    pub wmap_bytes_raw_named: usize,
    pub wmap_bytes_unexplained: usize,
    pub wmap_failures: usize,
    pub blocks: Vec<BlockCellReport>,
    pub failures: Vec<String>,
}

impl WorldMapReport {
    /// Counts-only view for `audit/world-map-inventory.json` (no per-block lists).
    pub fn audit_json(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).expect("report serializes");
        if let Some(map) = value.as_object_mut() {
            map.remove("blocks");
        }
        value
    }
}

fn pptr_of(value: &Value<'_>) -> Option<PPtr> {
    Some(PPtr {
        file_id: i32::try_from(value.field("m_FileID")?.as_i64()?).ok()?,
        path_id: value.field("m_PathID")?.as_i64()?,
    })
}

struct Context<'a, 'f> {
    file: &'f SerializedFile<'a>,
    source: &'f str,
    by_id: HashMap<i64, usize>,
    trees: HashMap<i32, Result<TypeTree, String>>,
}

impl Context<'_, '_> {
    fn object(&self, path_id: i64) -> Option<usize> {
        self.by_id.get(&path_id).copied()
    }

    fn tree(&mut self, type_id: i32) -> Result<&TypeTree, String> {
        let file = self.file;
        let source = self.source;
        self.trees
            .entry(type_id)
            .or_insert_with(|| {
                file.type_tree(type_id)
                    .ok_or_else(|| format!("no type tree for type {type_id}"))
                    .and_then(|raw| TypeTree::parse(&raw, source).map_err(|e| e.to_string()))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn read(&mut self, index: usize) -> Result<Value<'_>, String> {
        let file = self.file;
        let source = self.source;
        let type_id = file.report().objects[index].type_id;
        let data = file.object_data(index).map_err(|e| e.to_string())?;
        let tree = self.tree(type_id)?;
        tree.read(data, source).map_err(|e| e.to_string())
    }

    fn decode(&mut self, index: usize) -> Result<UnityObject, String> {
        let file = self.file;
        let source = self.source;
        let type_id = file.report().objects[index].type_id;
        let tree = self.tree(type_id)?;
        decode_object(file, tree, index, source).map_err(|e| e.to_string())
    }
}

struct MeshInfo {
    name: String,
    vertices: u32,
    triangles: u32,
    submeshes: usize,
    non_triangle_submeshes: usize,
    has_tangent: bool,
    walk_ids: Vec<u32>,
    bad_walk_ids: usize,
}

fn mesh_info(mesh: &crate::pc::unity_object::Mesh) -> MeshInfo {
    let tangents = mesh
        .channel_f32(7)
        .filter(|data| data.len() == mesh.vertex_count as usize * 4);
    let mut triangles = 0_u32;
    let mut non_triangle = 0;
    let mut ids = BTreeSet::new();
    let mut bad = 0;
    for submesh in &mesh.submeshes {
        if submesh.topology != 0 {
            non_triangle += 1;
            continue;
        }
        triangles += submesh.index_count / 3;
        let start = submesh.first_byte as usize / 2;
        let indices = &mesh.indices[start..start + submesh.index_count as usize];
        if let Some(tangents) = &tangents {
            for triangle in indices.chunks_exact(3) {
                let x = tangents[usize::from(triangle[0]) * 4];
                if x >= 0.0 && x.fract() == 0.0 && x < 4_294_967_000.0 {
                    ids.insert(x as u32);
                } else {
                    bad += 1;
                }
            }
        }
    }
    MeshInfo {
        name: mesh.name.clone(),
        vertices: mesh.vertex_count,
        triangles,
        submeshes: mesh.submeshes.len(),
        non_triangle_submeshes: non_triangle,
        has_tangent: tangents.is_some(),
        walk_ids: ids.into_iter().collect(),
        bad_walk_ids: bad,
    }
}

struct PrefabRaw {
    fields: BTreeMap<String, i64>,
    slots: Vec<(String, String, Option<i64>)>,
}

/// Reads one block prefab: the `WMBlockPrefab` fields and, for each non-null
/// Transform slot, the child object's name and its `MeshFilter` mesh path id.
fn read_prefab(context: &mut Context<'_, '_>, root_path_id: i64) -> Result<PrefabRaw, String> {
    let root_index = context
        .object(root_path_id)
        .ok_or("prefab root object is missing")?;
    let UnityObject::GameObject(root) = context.decode(root_index)? else {
        return Err("prefab root is not a GameObject".to_owned());
    };
    let mut behaviour = None;
    for (class, pptr) in &root.components {
        if *class != i32::from(MONO_BEHAVIOUR) || !pptr.is_local() {
            continue;
        }
        let index = context.object(pptr.path_id).ok_or("component is missing")?;
        let value = context.read(index)?;
        let script = value.field("m_Script").and_then(pptr_of);
        let class_name = script
            .filter(PPtr::is_local)
            .and_then(|script| context.object(script.path_id))
            .filter(|script| context.file.report().objects[*script].class_id == MONO_SCRIPT)
            .map(|script| {
                context.read(script).ok().and_then(|value| {
                    value
                        .field("m_ClassName")
                        .and_then(|v| v.as_str().map(str::to_owned))
                })
            });
        if class_name.flatten().as_deref() == Some(WM_BLOCK_PREFAB_CLASS) {
            behaviour = Some(index);
            break;
        }
    }
    let behaviour = behaviour.ok_or("prefab has no WMBlockPrefab component")?;
    let value = context.read(behaviour)?;
    let Value::Struct(members) = &value else {
        return Err("WMBlockPrefab is not a struct".to_owned());
    };
    let mut fields = BTreeMap::new();
    let mut slot_ptrs = Vec::new();
    for (name, member) in members {
        if matches!(*name, "m_GameObject" | "m_Enabled" | "m_Script" | "m_Name") {
            continue;
        }
        if let Some(pptr) = pptr_of(member) {
            if !pptr.is_null() {
                slot_ptrs.push(((*name).to_owned(), pptr));
            }
        } else if let Some(number) = member.as_i64() {
            fields.insert((*name).to_owned(), number);
        }
    }
    let mut slots = Vec::new();
    for (slot, pptr) in slot_ptrs {
        let transform_index = context
            .object(pptr.path_id)
            .ok_or("slot Transform is missing")?;
        let UnityObject::Transform(transform) = context.decode(transform_index)? else {
            return Err(format!("slot {slot} is not a Transform"));
        };
        let transform: Transform = transform;
        let go_index = context
            .object(transform.game_object.path_id)
            .ok_or("slot GameObject is missing")?;
        let UnityObject::GameObject(object) = context.decode(go_index)? else {
            return Err(format!("slot {slot} object is not a GameObject"));
        };
        let object: GameObject = object;
        let mut mesh = None;
        for (class, component) in &object.components {
            if *class == i32::from(MESH_FILTER) {
                let index = context
                    .object(component.path_id)
                    .ok_or("MeshFilter missing")?;
                let value = context.read(index)?;
                mesh = value.field("m_Mesh").and_then(pptr_of).map(|p| p.path_id);
            }
        }
        slots.push((slot, object.name, mesh));
    }
    Ok(PrefabRaw { fields, slots })
}

pub fn analyze_world_map(
    file: &SerializedFile<'_>,
    source: &str,
) -> Result<WorldMapReport, ImportError> {
    let objects = &file.report().objects;
    let by_id: HashMap<i64, usize> = objects
        .iter()
        .enumerate()
        .map(|(index, object)| (object.path_id, index))
        .collect();
    let bundle_index = objects
        .iter()
        .position(|object| object.class_id == 142)
        .ok_or_else(|| ImportError::invalid(source, "AssetBundle metadata object is missing"))?;
    let metadata = AssetBundleMetadata::parse(file.object_data(bundle_index)?, source)?;
    let mut class_counts = BTreeMap::new();
    for object in objects {
        *class_counts.entry(object.class_id).or_insert(0) += 1;
    }
    let mut context = Context {
        file,
        source,
        by_id,
        trees: HashMap::new(),
    };

    let mut report = WorldMapReport {
        bundle_object_count: objects.len(),
        class_counts,
        container_entries: metadata.container.len(),
        distinct_container_paths: 0,
        classified_entries: 0,
        unclassified_entries: 0,
        unclassified_samples: Vec::new(),
        groups: Vec::new(),
        block_mesh_folders: BTreeMap::new(),
        block_meshes_decoded: 0,
        block_meshes_failed: 0,
        block_mesh_vertices: 0,
        block_mesh_triangles: 0,
        block_mesh_name_path_mismatches: 0,
        block_mesh_row_mismatches: 0,
        discs: Vec::new(),
        objects: ObjectSummary::default(),
        wmap_assets: Vec::new(),
        wmap_bytes_total: 0,
        wmap_bytes_parsed: 0,
        wmap_bytes_zero_padding: 0,
        wmap_bytes_raw_named: 0,
        wmap_bytes_unexplained: 0,
        wmap_failures: 0,
        blocks: Vec::new(),
        failures: Vec::new(),
    };
    let mut groups: BTreeMap<(String, u16), usize> = BTreeMap::new();
    let mut cells: BTreeMap<(u32, u32, u32), BlockCellReport> = BTreeMap::new();
    let mut mesh_paths: HashMap<i64, (String, u32, u32, u32)> = HashMap::new();
    let mut prefab_roots: Vec<(String, i64, u32, u32, u32, String)> = Vec::new();
    let mut seen_paths = BTreeSet::new();

    for (path, info) in &metadata.container {
        seen_paths.insert(path.clone());
        let Some(&index) = context.by_id.get(&info.path_id) else {
            report.unclassified_entries += 1;
            report
                .failures
                .push(format!("{path}: object {} is missing", info.path_id));
            continue;
        };
        let class_id = objects[index].class_id;
        let Some((group, detail)) = classify_entry(path, class_id) else {
            report.unclassified_entries += 1;
            if report.unclassified_samples.len() < SAMPLE_LIMIT {
                report
                    .unclassified_samples
                    .push(format!("{path} (class {class_id})"));
            }
            continue;
        };
        report.classified_entries += 1;
        *groups.entry((group.clone(), class_id)).or_insert(0) += 1;

        match (class_id, &detail) {
            (MESH, Some((disc, row, form, name))) => {
                let folder = path
                    .strip_prefix(PREFIX)
                    .and_then(|rest| rest.split('/').nth(1))
                    .unwrap_or("")
                    .to_owned();
                *report.block_mesh_folders.entry(folder).or_insert(0) += 1;
                if *row != name.y {
                    report.block_mesh_row_mismatches += 1;
                }
                mesh_paths.insert(info.path_id, (path.clone(), *disc, name.x, name.y));
                match context.decode(index) {
                    Ok(UnityObject::Mesh(mesh)) => {
                        let mesh_data = mesh_info(&mesh);
                        report.block_meshes_decoded += 1;
                        let stem = path.rsplit('/').next().unwrap_or("");
                        let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
                        let name_matches = mesh_data.name.eq_ignore_ascii_case(stem);
                        if !name_matches {
                            report.block_mesh_name_path_mismatches += 1;
                        }
                        if mesh_data.bad_walk_ids > 0 {
                            report.failures.push(format!(
                                "{path}: {} triangles with a non-integer walk id",
                                mesh_data.bad_walk_ids
                            ));
                        }
                        report.block_mesh_vertices += u64::from(mesh_data.vertices);
                        report.block_mesh_triangles += u64::from(mesh_data.triangles);
                        let cell = cells.entry((*disc, name.x, name.y)).or_insert_with(|| {
                            BlockCellReport {
                                disc: *disc,
                                x: name.x,
                                y: name.y,
                                meshes: Vec::new(),
                                prefab: None,
                                variant_prefabs: Vec::new(),
                                vertices: 0,
                                triangles: 0,
                            }
                        });
                        cell.vertices += u64::from(mesh_data.vertices);
                        cell.triangles += u64::from(mesh_data.triangles);
                        cell.meshes.push(BlockMeshReport {
                            form: *form,
                            kind: name.kind.clone(),
                            path: path.clone(),
                            vertices: mesh_data.vertices,
                            triangles: mesh_data.triangles,
                            submeshes: mesh_data.submeshes,
                            non_triangle_submeshes: mesh_data.non_triangle_submeshes,
                            has_tangent_channel: mesh_data.has_tangent,
                            walk_ids: mesh_data.walk_ids,
                            mesh_name_matches_path: name_matches,
                        });
                    }
                    Ok(_) => {
                        report.block_meshes_failed += 1;
                        report
                            .failures
                            .push(format!("{path}: not decoded as a Mesh"));
                    }
                    Err(message) => {
                        report.block_meshes_failed += 1;
                        report.failures.push(format!("{path}: {message}"));
                    }
                }
            }
            (GAME_OBJECT, Some((disc, row, _, name))) => {
                if *row != name.y {
                    report
                        .failures
                        .push(format!("{path}: row folder differs from block y"));
                }
                prefab_roots.push((
                    path.clone(),
                    info.path_id,
                    *disc,
                    name.x,
                    name.y,
                    name.kind.clone(),
                ));
            }
            _ => {}
        }
        if class_id == TEXT_ASSET && detail.is_none() {
            // handled below, after the main pass
        }
    }
    report.distinct_container_paths = seen_paths.len();
    report.groups = groups
        .into_iter()
        .map(|((group, class_id), count)| GroupCount {
            group,
            class_id,
            count,
        })
        .collect();

    // Prefabs.
    let mut mesh_users: HashMap<i64, usize> = HashMap::new();
    for (path, root_id, disc, x, y, variant) in &prefab_roots {
        let raw = match read_prefab(&mut context, *root_id) {
            Ok(raw) => raw,
            Err(message) => {
                report.failures.push(format!("{path}: {message}"));
                continue;
            }
        };
        let mut slots = Vec::new();
        for (slot, object_name, mesh_id) in raw.slots {
            let mesh_path = mesh_id.and_then(|id| mesh_paths.get(&id).cloned());
            if let Some(id) = mesh_id {
                *mesh_users.entry(id).or_insert(0) += 1;
            }
            let matches = mesh_path
                .as_ref()
                .is_some_and(|(_, d, mx, my)| d == disc && mx == x && my == y);
            let is_sea = raw.fields.get("IsSea").is_some_and(|value| *value != 0);
            let shared_sea_mesh = !matches
                && is_sea
                && mesh_path
                    .as_ref()
                    .is_some_and(|(_, _, mx, my)| *mx == 12 && *my == 0);
            slots.push(PrefabSlot {
                slot,
                object_name,
                mesh_path: mesh_path.map(|(path, ..)| path),
                mesh_matches_block: matches,
                shared_sea_mesh,
            });
        }
        let cell = cells
            .entry((*disc, *x, *y))
            .or_insert_with(|| BlockCellReport {
                disc: *disc,
                x: *x,
                y: *y,
                meshes: Vec::new(),
                prefab: None,
                variant_prefabs: Vec::new(),
                vertices: 0,
                triangles: 0,
            });
        let prefab = PrefabReport {
            prefab_path: path.clone(),
            variant: variant.clone(),
            slots,
            fields: raw.fields,
        };
        if variant.is_empty() {
            cell.prefab = Some(prefab);
        } else {
            cell.variant_prefabs.push(prefab);
        }
    }

    // map.bin, for the sea/land cross-check.
    let mut wmap_bytes: Vec<(String, Vec<u8>)> = Vec::new();
    for (path, info) in &metadata.container {
        if !path.starts_with(PREFIX) || !path.contains("/wmap/") {
            continue;
        }
        let Some(&index) = context.by_id.get(&info.path_id) else {
            continue;
        };
        if objects[index].class_id != TEXT_ASSET {
            continue;
        }
        match TextAsset::parse(file.object_data(index)?, path) {
            Ok(asset) => wmap_bytes.push((path.clone(), asset.script)),
            Err(error) => report.failures.push(format!("{path}: {error}")),
        }
    }
    let map_grid: Option<Vec<u8>> = wmap_bytes
        .iter()
        .find(|(path, _)| path.ends_with("/map.bin.bytes"))
        .map(|(_, bytes)| bytes.clone())
        .filter(|bytes| bytes.len() == MAP_GRID_WIDTH * MAP_GRID_HEIGHT);

    // Per-disc summaries.
    let mut discs: BTreeMap<u32, DiscSummary> = BTreeMap::new();
    for cell in cells.values() {
        let summary = discs.entry(cell.disc).or_insert_with(|| DiscSummary {
            disc: cell.disc,
            grid_min_x: u32::MAX,
            grid_min_y: u32::MAX,
            ..DiscSummary::default()
        });
        if !cell.meshes.is_empty() {
            summary.cells_with_meshes += 1;
        }
        summary.block_meshes += cell.meshes.len();
        summary.vertices += cell.vertices;
        summary.triangles += cell.triangles;
        summary.grid_min_x = summary.grid_min_x.min(cell.x);
        summary.grid_max_x = summary.grid_max_x.max(cell.x);
        summary.grid_min_y = summary.grid_min_y.min(cell.y);
        summary.grid_max_y = summary.grid_max_y.max(cell.y);
        for mesh in &cell.meshes {
            *summary
                .meshes_by_kind
                .entry(
                    mesh.kind
                        .trim_end_matches(|c: char| c.is_ascii_digit())
                        .to_owned(),
                )
                .or_insert(0) += 1;
        }
        if let Some(prefab) = &cell.prefab {
            summary.block_prefabs += 1;
            if prefab.slots.iter().all(|slot| slot.mesh_path.is_none()) {
                summary.prefabs_without_meshes += 1;
            }
            summary.prefab_slots += prefab.slots.len();
            summary.prefab_slots_matching_block += prefab
                .slots
                .iter()
                .filter(|slot| slot.mesh_matches_block)
                .count();
            summary.prefab_slots_shared_sea_mesh += prefab
                .slots
                .iter()
                .filter(|slot| slot.shared_sea_mesh)
                .count();
            for (name, value) in &prefab.fields {
                if !(name.starts_with("Has") || name.starts_with("Is")) {
                    continue;
                }
                *summary.prefab_flag_totals.entry(name.clone()).or_insert(0) += value;
            }
            let initial_x = prefab.fields.get("InitialX");
            let initial_y = prefab.fields.get("InitialY");
            if initial_x.is_some_and(|v| *v != i64::from(cell.x))
                || initial_y.is_some_and(|v| *v != i64::from(cell.y))
            {
                summary.prefab_initial_xy_mismatch += 1;
            }
            let in_grid = (cell.x as usize) < MAP_GRID_WIDTH && (cell.y as usize) < MAP_GRID_HEIGHT;
            if let (1, Some(grid), true) = (cell.disc, &map_grid, in_grid) {
                let value = grid[cell.y as usize * MAP_GRID_WIDTH + cell.x as usize];
                let sea = prefab.fields.get("IsSea").is_some_and(|v| *v != 0);
                let target = if sea {
                    &mut summary.map_values_for_sea_cells
                } else {
                    &mut summary.map_values_for_land_cells
                };
                *target.entry(value).or_insert(0) += 1;
            }
        }
    }
    for (path_id, (mesh_path, disc, ..)) in &mesh_paths {
        let users = mesh_users.get(path_id).copied().unwrap_or(0);
        if let Some(summary) = discs.get_mut(disc) {
            if users == 0 {
                summary.meshes_not_in_any_prefab += 1;
                let stem = mesh_path.rsplit('/').next().unwrap_or("").to_lowercase();
                let stem = stem.rsplit_once('.').map_or(stem.as_str(), |(s, _)| s);
                let kind = parse_block_name(stem)
                    .map(|name| name.kind)
                    .unwrap_or_default();
                let form = mesh_path
                    .split('/')
                    .nth(4)
                    .and_then(|folder| folder.strip_prefix("0_"))
                    .unwrap_or("?");
                let at = parse_block_name(stem)
                    .map(|name| format!("form{form} [{}][{}]", name.x, name.y));
                *summary
                    .unreferenced_mesh_kinds
                    .entry(format!(
                        "{}{}",
                        at.unwrap_or_default(),
                        if kind.is_empty() {
                            String::new()
                        } else {
                            format!(" {kind}")
                        }
                    ))
                    .or_insert(0) += 1;
            } else if users > 1 {
                summary.meshes_in_more_than_one_prefab += 1;
            }
        }
    }
    report.discs = discs.into_values().collect();

    // Other Unity objects named in the container.
    for (path, info) in &metadata.container {
        let Some(&index) = context.by_id.get(&info.path_id) else {
            continue;
        };
        let class_id = objects[index].class_id;
        if mesh_paths.contains_key(&info.path_id) {
            continue;
        }
        match class_id {
            MATERIAL => {
                report.objects.materials += 1;
                if let Ok(UnityObject::Material(material)) = context.decode(index) {
                    report.objects.materials_decoded += 1;
                    report.objects.material_texture_slots += material.textures.len();
                } else {
                    report
                        .failures
                        .push(format!("{path}: Material did not decode"));
                }
            }
            TEXTURE_2D => {
                report.objects.textures += 1;
                match context.read(index) {
                    Ok(value) => {
                        let fields = Fields::new(&value, path, "Texture2D");
                        let width = fields.u32("m_Width").unwrap_or(0);
                        let height = fields.u32("m_Height").unwrap_or(0);
                        let format = fields.i32("m_TextureFormat");
                        if let (true, Ok(format)) = (width > 0 && height > 0, format) {
                            report.objects.textures_decoded += 1;
                            *report
                                .objects
                                .texture_formats
                                .entry(format.to_string())
                                .or_insert(0) += 1;
                            report.objects.texture_max_dimension =
                                report.objects.texture_max_dimension.max(width.max(height));
                        } else {
                            report
                                .failures
                                .push(format!("{path}: Texture2D header unreadable"));
                        }
                    }
                    Err(message) => report.failures.push(format!("{path}: {message}")),
                }
            }
            RENDER_TEXTURE => report.objects.render_textures += 1,
            MESH => {
                report.objects.other_meshes += 1;
                if matches!(context.decode(index), Ok(UnityObject::Mesh(_))) {
                    report.objects.other_meshes_decoded += 1;
                } else {
                    report.failures.push(format!("{path}: Mesh did not decode"));
                }
            }
            GAME_OBJECT => {
                report.objects.game_objects += 1;
                if matches!(context.decode(index), Ok(UnityObject::GameObject(_))) {
                    report.objects.game_objects_decoded += 1;
                }
            }
            _ => {}
        }
    }

    // wmap binaries.
    for (path, bytes) in &wmap_bytes {
        let asset = analyze_wmap_asset(path, bytes);
        report.wmap_bytes_total += asset.bytes;
        report.wmap_bytes_parsed += asset.account.parsed;
        report.wmap_bytes_zero_padding += asset.account.zero_padding;
        report.wmap_bytes_raw_named += asset.account.raw_named;
        report.wmap_bytes_unexplained += asset.account.unexplained;
        if !asset.parsed_ok {
            report.wmap_failures += 1;
        }
        report.wmap_assets.push(asset);
    }
    report.blocks = cells.into_values().collect();
    Ok(report)
}

/// Walk-mesh inputs of one block mesh, as `WMBlock.AddWalkMesh` reads them
/// (`mesh.vertices`, `mesh.triangles`, `mesh.tangents`; `Mesh.triangles` adds each
/// submesh's base vertex).
#[derive(Debug, Clone, PartialEq)]
pub struct WalkMeshData {
    pub path: String,
    /// Block-local vertex positions in Unity units.
    pub positions: Vec<[f32; 3]>,
    /// Triangle vertex indices, three per triangle, submeshes in order.
    pub indices: Vec<u32>,
    /// `tangent.x` of every vertex (the per-triangle map id is read from a
    /// triangle's first vertex).
    pub tangent_x: Vec<f32>,
}

/// Decodes the block meshes whose container path satisfies `wanted` into walk-mesh
/// inputs. Meshes without a tangent channel are skipped and listed in the second
/// return value.
pub fn extract_walk_meshes(
    file: &SerializedFile<'_>,
    source: &str,
    wanted: &dyn Fn(&str) -> bool,
) -> Result<(Vec<WalkMeshData>, Vec<String>), ImportError> {
    let objects = &file.report().objects;
    let by_id: HashMap<i64, usize> = objects
        .iter()
        .enumerate()
        .map(|(index, object)| (object.path_id, index))
        .collect();
    let bundle_index = objects
        .iter()
        .position(|object| object.class_id == 142)
        .ok_or_else(|| ImportError::invalid(source, "AssetBundle metadata object is missing"))?;
    let metadata = AssetBundleMetadata::parse(file.object_data(bundle_index)?, source)?;
    let mut context = Context {
        file,
        source,
        by_id,
        trees: HashMap::new(),
    };
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for (path, info) in &metadata.container {
        if !wanted(path) {
            continue;
        }
        let Some(index) = context.object(info.path_id) else {
            skipped.push(format!("{path}: object missing"));
            continue;
        };
        if objects[index].class_id != MESH {
            continue;
        }
        let mesh = match context.decode(index) {
            Ok(UnityObject::Mesh(mesh)) => mesh,
            Ok(_) => {
                skipped.push(format!("{path}: not a Mesh"));
                continue;
            }
            Err(message) => {
                skipped.push(format!("{path}: {message}"));
                continue;
            }
        };
        let count = mesh.vertex_count as usize;
        let positions = mesh.channel_f32(0).filter(|data| data.len() == count * 3);
        let tangents = mesh.channel_f32(7).filter(|data| data.len() == count * 4);
        let (Some(positions), Some(tangents)) = (positions, tangents) else {
            skipped.push(format!("{path}: no position or tangent channel"));
            continue;
        };
        let mut indices = Vec::new();
        for submesh in &mesh.submeshes {
            if submesh.topology != 0 {
                continue;
            }
            let start = submesh.first_byte as usize / 2;
            let end = start + submesh.index_count as usize;
            let Some(slice) = mesh.indices.get(start..end) else {
                skipped.push(format!("{path}: submesh index range out of bounds"));
                continue;
            };
            indices.extend(slice.iter().map(|i| u32::from(*i) + submesh.first_vertex));
        }
        out.push(WalkMeshData {
            path: path.clone(),
            positions: positions
                .chunks_exact(3)
                .map(|p| [p[0], p[1], p[2]])
                .collect(),
            indices,
            tangent_x: tangents.chunks_exact(4).map(|t| t[0]).collect(),
        });
    }
    Ok((out, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_names_parse() {
        let name = parse_block_name("block[10][3] sea4f").expect("name");
        assert_eq!((name.x, name.y, name.kind.as_str()), (10, 3, "sea4f"));
        let prefab = parse_block_name("block[0][19]").expect("prefab");
        assert_eq!((prefab.x, prefab.y, prefab.kind.as_str()), (0, 19, ""));
        assert_eq!(
            parse_block_name("block[12][0]f").expect("variant").kind,
            "f"
        );
        assert!(parse_block_name("block[1][2]xyz").is_none());
        assert!(parse_block_name("blocks[1][2]").is_none());
        assert!(parse_block_name("block[a][2]").is_none());
    }

    #[test]
    fn classification_rules() {
        let mesh = "assets/resources/worldmap/disc1/0_1/r3/block[5][3] terrain.asset";
        let (group, detail) = classify_entry(mesh, MESH).expect("mesh");
        assert_eq!(group, "disc1/block_mesh");
        let (disc, row, _, name) = detail.expect("detail");
        assert_eq!((disc, row, name.x, name.y), (1, 3, 5, 3));
        // Wrong class for the path is unclassified, not silently accepted.
        assert!(classify_entry(mesh, MATERIAL).is_none());
        let prefab = "assets/resources/worldmap/prefabs/worlddisc4/r0/block[0][0].prefab";
        assert_eq!(
            classify_entry(prefab, GAME_OBJECT).expect("prefab").0,
            "prefabs/disc4/block_prefab"
        );
        assert_eq!(
            classify_entry("assets/resources/worldmap/wmap/map.bin.bytes", TEXT_ASSET)
                .expect("wmap")
                .0,
            "wmap/binary_text_asset"
        );
        assert!(classify_entry("assets/resources/other/x.asset", MESH).is_none());
    }
}
