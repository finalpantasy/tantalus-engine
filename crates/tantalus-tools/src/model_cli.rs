//! `tantalusctl assets inspect-bundle --assemble-model / --assemble-all`.

use std::path::{Path, PathBuf};
use tantalus_assets::pc::model_assembly::pose::Interpolation;
use tantalus_assets::pc::model_assembly::{
    AssembleOptions, ModelCatalog, SerializedStore, assemble_all, assemble_model, clip_entries,
    clip_report, embedded_clip_report,
};
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::unity_raw::UnityRawBundle;

const MAX_BUNDLE_BYTES: usize = 600 * 1024 * 1024;

/// Which model report to run, from the command line.
pub struct ModelRequest {
    /// `<type>/<geo id>`, a geo id or a geo name.
    pub model: Option<String>,
    pub all: bool,
    /// Animation bundle for the clip check; defaults to a sibling
    /// `p0data5.bin`.
    pub clips: Option<PathBuf>,
    pub json: bool,
}

pub fn run_model_request(
    bundle: &UnityRawBundle,
    path: &Path,
    request: &ModelRequest,
) -> Result<(), String> {
    let source = path.to_string_lossy().into_owned();
    let bytes = bundle
        .read_entry(0, MAX_BUNDLE_BYTES)
        .map_err(|error| error.to_string())?;
    let file = SerializedFile::parse(&bytes, &source).map_err(|error| error.to_string())?;
    let store = SerializedStore::new(&file, &source).map_err(|error| error.to_string())?;
    let catalog = ModelCatalog::from_container(&store.container().map_err(|e| e.to_string())?);
    if let Some(query) = &request.model {
        return run_one(&store, &catalog, query, request.json);
    }
    run_all(&store, &catalog, path, request)
}

fn run_one(
    store: &SerializedStore<'_, '_>,
    catalog: &ModelCatalog,
    query: &str,
    json: bool,
) -> Result<(), String> {
    let dir = catalog.find(query)?;
    let root = *dir
        .roots
        .first()
        .ok_or_else(|| format!("directory {} has no root GameObject", dir.label))?;
    let model = assemble_model(store, &dir.label, root, AssembleOptions::default())
        .map_err(|error| error.to_string())?;
    let summary = model.summary();
    if json {
        let nodes: Vec<_> = model
            .nodes
            .iter()
            .map(|node| {
                serde_json::json!({
                    "name": node.name,
                    "path": node.path,
                    "parent": node.parent,
                    "position": node.local_position,
                    "rotation": node.local_rotation,
                    "scale": node.local_scale,
                })
            })
            .collect();
        let parts: Vec<_> = model
            .parts
            .iter()
            .map(|part| {
                serde_json::json!({
                    "name": part.name,
                    "node": part.node,
                    "mesh": part.mesh_name,
                    "vertices": part.positions.len(),
                    "has_normals": !part.normals.is_empty(),
                    "has_uvs": !part.uvs.is_empty(),
                    "indices": part.indices.len(),
                    "bones": part.bones,
                    "root_bone": part.root_bone,
                    "submeshes": part.submeshes.iter().map(|submesh| serde_json::json!({
                        "first_index": submesh.first_index,
                        "index_count": submesh.index_count,
                        "topology": submesh.topology,
                        "material": submesh.material.as_ref().map(|material| serde_json::json!({
                            "name": material.name,
                            "textures": material.textures.iter().map(|texture| serde_json::json!({
                                "property": texture.property,
                                "name": texture.texture_name,
                                "size": [texture.width, texture.height],
                                "format": texture.format,
                                "decoded_bytes": texture.rgba.as_ref().map(Vec::len),
                                "undecodable": texture.undecodable,
                            })).collect::<Vec<_>>(),
                        })),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        let report = serde_json::json!({
            "directory": dir,
            "summary": summary,
            "nodes": nodes,
            "parts": parts,
        });
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return Ok(());
    }
    println!(
        "{} ({} {}): {} nodes (depth {}), {} skinned parts, {} vertices, {} distinct bones, {} bone failures",
        dir.label,
        dir.type_name,
        dir.geo_name.unwrap_or("unnamed geo id"),
        summary.nodes,
        summary.max_depth,
        summary.parts,
        summary.vertices,
        summary.distinct_bones,
        summary.bone_resolution_failures,
    );
    println!(
        "  {} materials, {} textures ({} decoded, {} undecodable), {} embedded clips, rest skin error {:.2e}",
        summary.materials,
        summary.textures,
        summary.textures_decoded,
        summary.textures_undecodable,
        summary.embedded_clips,
        summary.rest_skin_error,
    );
    for failure in &summary.texture_failures {
        println!("  undecodable texture: {failure}");
    }
    for sample in &summary.issues.samples {
        println!("  issue: {sample}");
    }
    Ok(())
}

fn run_all(
    store: &SerializedStore<'_, '_>,
    catalog: &ModelCatalog,
    path: &Path,
    request: &ModelRequest,
) -> Result<(), String> {
    let (mut report, models) = assemble_all(store, catalog, AssembleOptions::default());
    let embedded = embedded_clip_report(store, &models, Interpolation::Hermite);
    let clip_path = request
        .clips
        .clone()
        .or_else(|| Some(path.with_file_name("p0data5.bin")).filter(|p| p.is_file()));
    let clips = match &clip_path {
        Some(clip_path) => Some(run_clips(clip_path, catalog, &models)?),
        None => None,
    };
    if request.json {
        let output = serde_json::json!({
            "models": report,
            "embedded_clips": embedded,
            "clips": clips,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
        return Ok(());
    }
    let listing = std::mem::take(&mut report.models);
    println!(
        "model directories: {} ({} without a root GameObject), assembled: {}, failed: {}",
        report.model_directories,
        report.directories_without_root,
        report.assembled,
        report.failed.len()
    );
    for failure in report.failed.iter().take(10) {
        println!("  {failure}");
    }
    println!(
        "nodes {}, skinned parts {}, vertices {}, distinct bones {} ({} bone slots)",
        report.nodes,
        report.skinned_parts,
        report.vertices,
        report.distinct_bones,
        report.bone_slots
    );
    println!(
        "bone-to-node failures {}, bone/bind-pose count mismatches {}, mesh unresolved {}, material unresolved {}, texture unresolved {}, hierarchy unresolved {}",
        report.bone_resolution_failures,
        report.bone_count_mismatches,
        report.mesh_unresolved,
        report.material_unresolved,
        report.texture_unresolved,
        report.hierarchy_unresolved
    );
    println!(
        "materials {}, textures {} ({} decoded, {} undecodable), max rest skin error {:.2e}",
        report.materials,
        report.textures,
        report.textures_decoded,
        report.textures_undecodable,
        report.max_rest_skin_error
    );
    for (reason, count) in &report.texture_failure_reasons {
        println!("  undecodable x{count}: {reason}");
    }
    for label in &report.rootless_directories {
        println!("  no root GameObject: {label}");
    }
    for (label, error) in report.rest_skin_outliers.iter().take(8) {
        println!("  rest skin outlier {label}: {error:.3}");
    }
    println!("model types: {:?}", report.model_types);
    println!(
        "geo ids with a Memoria name: {} of {}; container entries outside model dirs: {}",
        report.geo_ids_with_name, report.model_directories, report.other_container_entries
    );
    println!(
        "unreached objects by class (orphans): {:?}",
        report.unreached_objects
    );
    drop(listing);
    println!("{}", serde_json::to_string_pretty(&embedded).unwrap());
    if let Some(clips) = clips {
        println!("{}", serde_json::to_string_pretty(&clips).unwrap());
    }
    Ok(())
}

fn run_clips(
    clip_path: &Path,
    catalog: &ModelCatalog,
    models: &[(
        tantalus_assets::pc::model_assembly::ModelDir,
        tantalus_assets::pc::model_assembly::AssembledModel,
    )],
) -> Result<tantalus_assets::pc::model_assembly::ClipReport, String> {
    let source = clip_path.to_string_lossy().into_owned();
    let bundle = UnityRawBundle::open(clip_path).map_err(|error| error.to_string())?;
    let bytes = bundle
        .read_entry(0, MAX_BUNDLE_BYTES)
        .map_err(|error| error.to_string())?;
    let file = SerializedFile::parse(&bytes, &source).map_err(|error| error.to_string())?;
    let store = SerializedStore::new(&file, &source).map_err(|error| error.to_string())?;
    let (clips, other) = clip_entries(&store.container().map_err(|e| e.to_string())?);
    if !other.is_empty() {
        eprintln!(
            "{} animation container entries did not match animations/<geo>/<name>.anim",
            other.len()
        );
    }
    Ok(clip_report(
        &store,
        &clips,
        catalog,
        models,
        Interpolation::Hermite,
    ))
}
