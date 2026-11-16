use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tantalus_assets::{
    CancellationToken, DirectoryAssetSource, ImportPlan, import_subset, inspect_install,
};

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("tantalusctl: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<OsString>) -> Result<(), String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }
    if args.len() < 2 {
        return Err(usage().to_owned());
    }
    match (args[0].to_str(), args[1].to_str()) {
        (Some("install"), Some("inspect")) => run_install_inspect(args),
        (Some("audit"), Some("summary")) => run_audit_summary(args),
        (Some("audit"), Some("compare-sweeps")) => run_compare_sweeps(args),
        (Some("assets"), Some("import")) => run_assets_import(args),
        (Some("assets"), Some("inspect-bundle")) => run_assets_inspect_bundle(args),
        (Some("assets"), Some("extract-resource")) => run_assets_extract_resource(args),
        (Some("assets"), Some("extract-text")) => run_assets_extract_text(args),
        (Some("assets"), Some("battle-tables")) => run_assets_battle_tables(args),
        (Some("assets"), Some("inspect-serialized")) => run_assets_inspect_serialized(args),
        (Some("assets"), Some("inspect-movie")) => run_assets_inspect_movie(args),
        (Some("scenario"), Some("run")) => run_scenario(args),
        _ => Err(usage().to_owned()),
    }
}

fn run_assets_extract_resource(args: Vec<OsString>) -> Result<(), String> {
    let mut dll = None;
    let mut name = None;
    let mut output = None;
    let mut json = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].to_str() {
            Some("--dll") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--dll` requires a path".to_owned())?;
                if dll.replace(PathBuf::from(value)).is_some() {
                    return Err("`--dll` may only be supplied once".to_owned());
                }
            }
            Some("--name") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--name` requires a UTF-8 resource name".to_owned())?;
                if name.replace(value.to_owned()).is_some() {
                    return Err("`--name` may only be supplied once".to_owned());
                }
            }
            Some("--output") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--output` requires a path".to_owned())?;
                if output.replace(PathBuf::from(value)).is_some() {
                    return Err("`--output` may only be supplied once".to_owned());
                }
            }
            Some("--json") => json = true,
            _ => {
                return Err(format!(
                    "unrecognized argument: {}",
                    args[index].to_string_lossy()
                ));
            }
        }
        index += 1;
    }
    let dll = dll.ok_or_else(|| "missing required `--dll <path>`".to_owned())?;
    let name = name.ok_or_else(|| "missing required `--name <resource>`".to_owned())?;
    let bytes =
        std::fs::read(&dll).map_err(|error| format!("cannot read {}: {error}", dll.display()))?;
    let data = tantalus_assets::pc::managed::extract_resource(&bytes, &name)
        .map_err(|error| error.to_string())?;
    let digest = format!("{:x}", Sha256::digest(&data));
    let written_to = match &output {
        Some(target) => {
            std::fs::write(target, &data)
                .map_err(|error| format!("cannot write {}: {error}", target.display()))?;
            Some(target.to_string_lossy().into_owned())
        }
        None => None,
    };
    let report = serde_json::json!({
        "dll": dll.file_name().map(|n| n.to_string_lossy().into_owned()),
        "name": name,
        "bytes": data.len(),
        "sha256": digest,
        "written_to": written_to,
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!("{name}: {} bytes", data.len());
    }
    Ok(())
}

/// `assets battle-tables`: evaluate the installed managed assembly's static
/// initializers and parse the vanilla battle, item, ability, shop and level tables.
fn run_assets_battle_tables(args: Vec<OsString>) -> Result<(), String> {
    use tantalus_assets::pc::battle_tables::{extract_battle_tables, summarize, verify_known_rows};
    use tantalus_assets::pc::managed_il::Assembly;
    let mut dll = None;
    let mut out = None;
    let mut tables_out = None;
    let mut json = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].to_str() {
            Some("--json") => json = true,
            Some(flag @ ("--dll" | "--out" | "--tables-out")) => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{flag}` requires a path"))?;
                let slot_taken = match flag {
                    "--dll" => dll.replace(PathBuf::from(value)).is_some(),
                    "--out" => out.replace(PathBuf::from(value)).is_some(),
                    _ => tables_out.replace(PathBuf::from(value)).is_some(),
                };
                if slot_taken {
                    return Err(format!("`{flag}` may only be supplied once"));
                }
            }
            _ => {
                return Err(format!(
                    "unrecognized argument: {}",
                    args[index].to_string_lossy()
                ));
            }
        }
        index += 1;
    }
    let dll = dll.ok_or_else(|| "missing required `--dll <path>`".to_owned())?;
    let bytes =
        std::fs::read(&dll).map_err(|error| format!("cannot read {}: {error}", dll.display()))?;
    let assembly = Assembly::parse(&bytes).map_err(|error| error.to_string())?;
    let tables = extract_battle_tables(&assembly).map_err(|error| error.to_string())?;
    let checks = verify_known_rows(&tables);
    let report = summarize(&tables, &checks, &format!("{:x}", Sha256::digest(&bytes)));
    let text = serde_json::to_string_pretty(&report).unwrap();
    if let Some(target) = &out {
        std::fs::write(
            target,
            format!(
                "{text}
"
            ),
        )
        .map_err(|error| format!("cannot write {}: {error}", target.display()))?;
    }
    if let Some(target) = &tables_out {
        let full = serde_json::to_string(&tables).unwrap();
        std::fs::write(target, full)
            .map_err(|error| format!("cannot write {}: {error}", target.display()))?;
    }
    if json {
        println!("{text}");
    } else {
        for table in report["tables"].as_array().into_iter().flatten() {
            println!("{}: {} rows", table["name"], table["count"]);
        }
        let failed = checks.iter().filter(|check| !check.passed).count();
        println!("known-row checks: {} run, {failed} failed", checks.len());
    }
    if checks.iter().any(|check| !check.passed) {
        return Err("known-row checks failed".to_owned());
    }
    Ok(())
}

fn run_assets_extract_text(args: Vec<OsString>) -> Result<(), String> {
    let mut path = None;
    let mut name = None;
    let mut output = None;
    let mut json = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].to_str() {
            Some("--path") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--path` requires a path".to_owned())?;
                if path.replace(PathBuf::from(value)).is_some() {
                    return Err("`--path` may only be supplied once".to_owned());
                }
            }
            Some("--name") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--name` requires a UTF-8 asset name".to_owned())?;
                if name.replace(value.to_owned()).is_some() {
                    return Err("`--name` may only be supplied once".to_owned());
                }
            }
            Some("--output") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--output` requires a path".to_owned())?;
                if output.replace(PathBuf::from(value)).is_some() {
                    return Err("`--output` may only be supplied once".to_owned());
                }
            }
            Some("--json") => json = true,
            _ => {
                return Err(format!(
                    "unrecognized argument: {}",
                    args[index].to_string_lossy()
                ));
            }
        }
        index += 1;
    }
    let path = path.ok_or_else(|| "missing required `--path <file>`".to_owned())?;
    let name = name.ok_or_else(|| "missing required `--name <asset>`".to_owned())?;
    let bytes =
        std::fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let serialized =
        tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
            .map_err(|error| error.to_string())?;
    let mut found = None;
    let mut text_assets = 0usize;
    let mut sample_names = Vec::new();
    for (index, object) in serialized.report().objects.iter().enumerate() {
        if object.class_id != 49 {
            continue;
        }
        text_assets += 1;
        let asset = tantalus_assets::pc::text_asset::TextAsset::parse(
            serialized
                .object_data(index)
                .map_err(|error| error.to_string())?,
            &name,
        )
        .map_err(|error| error.to_string())?;
        if sample_names.len() < 8
            && (asset.name.contains("atan") || asset.name.contains("Embedded"))
        {
            sample_names.push(asset.name.clone());
        }
        if asset.name == name || asset.name.ends_with(&format!("/{name}")) {
            found = Some(asset.script);
            break;
        }
    }
    let data = found.ok_or_else(|| {
        format!("text asset `{name}` was not found (scanned {text_assets} text assets; atan/embedded samples: {sample_names:?})")
    })?;
    let digest = format!("{:x}", Sha256::digest(&data));
    let written_to = match &output {
        Some(target) => {
            std::fs::write(target, &data)
                .map_err(|error| format!("cannot write {}: {error}", target.display()))?;
            Some(target.to_string_lossy().into_owned())
        }
        None => None,
    };
    let report = serde_json::json!({
        "file": path.file_name().map(|n| n.to_string_lossy().into_owned()),
        "name": name,
        "bytes": data.len(),
        "sha256": digest,
        "written_to": written_to,
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!("{name}: {} bytes", data.len());
    }
    Ok(())
}

fn run_install_inspect(args: Vec<OsString>) -> Result<(), String> {
    let mut root = None;
    let mut json = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].to_str() {
            Some("--root") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--root` requires a path".to_owned())?;
                if root.replace(PathBuf::from(value)).is_some() {
                    return Err("`--root` may only be supplied once".to_owned());
                }
            }
            Some("--json") => json = true,
            _ => {
                return Err(format!(
                    "unrecognized argument: {}",
                    args[index].to_string_lossy()
                ));
            }
        }
        index += 1;
    }

    let root = root.ok_or_else(|| "missing required `--root <path>`".to_owned())?;
    let report = inspect_install(&root).map_err(|error| error.to_string())?;
    if json {
        let output = serde_json::to_string_pretty(&report)
            .map_err(|error| format!("could not serialize inspection report: {error}"))?;
        println!("{output}");
    } else {
        println!("Installation: {}", report.root);
        println!("Layout: {:?}", report.layout);
        println!("Build support: {:?}", report.build.support);
        println!("Reference runtime: {:?}", report.build.reference_runtime);
        println!(
            "Resources: {} present, {} required dependencies missing",
            report
                .resources
                .iter()
                .filter(
                    |resource| resource.status == tantalus_assets::install::ResourceStatus::Present
                )
                .count(),
            report.missing_dependencies.len()
        );
        for diagnostic in &report.diagnostics {
            println!("{}: {}", diagnostic.code, diagnostic.message);
        }
    }
    Ok(())
}

fn run_assets_import(args: Vec<OsString>) -> Result<(), String> {
    let mut plan_path = None;
    let mut json = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].to_str() {
            Some("--plan") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--plan` requires a path".to_owned())?;
                if plan_path.replace(PathBuf::from(value)).is_some() {
                    return Err("`--plan` may only be supplied once".to_owned());
                }
            }
            Some("--json") => json = true,
            _ => {
                return Err(format!(
                    "unrecognized argument: {}",
                    args[index].to_string_lossy()
                ));
            }
        }
        index += 1;
    }

    let plan_path = plan_path.ok_or_else(|| "missing required `--plan <path>`".to_owned())?;
    let bytes = std::fs::read(&plan_path)
        .map_err(|error| format!("could not read {}: {error}", plan_path.display()))?;
    let plan: ImportPlan = serde_json::from_slice(&bytes)
        .map_err(|error| format!("could not parse {}: {error}", plan_path.display()))?;
    if plan.source_root.as_os_str().is_empty() || plan.cache_root.as_os_str().is_empty() {
        return Err("the import plan must set `source_root` and `cache_root`".to_owned());
    }
    let source = DirectoryAssetSource::new(&plan.source_root);
    let manifest = import_subset(
        &plan,
        &source,
        &plan.cache_root,
        &CancellationToken::default(),
    )
    .map_err(|error| error.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| format!("could not serialize pack manifest: {error}"))?
        );
    } else {
        println!(
            "Imported {} assets into pack {}",
            manifest.assets.len(),
            manifest.pack_id
        );
    }
    Ok(())
}

fn run_unity_objects(
    bundle: &tantalus_assets::pc::unity_raw::UnityRawBundle,
    path: &std::path::Path,
    class_filter: Option<u16>,
    json: bool,
) -> Result<(), String> {
    let source = path.to_string_lossy();
    let bytes = bundle
        .read_entry(0, 600 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    let serialized = tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &source)
        .map_err(|error| error.to_string())?;
    let report = tantalus_assets::pc::unity_object::inspect_unity_objects(
        &serialized,
        class_filter,
        &source,
    )
    .map_err(|error| error.to_string())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return Ok(());
    }
    println!(
        "Unity {} type trees: {}; {} objects: {} parsed, {} failed",
        report.unity_version,
        report.type_trees_embedded,
        report.objects_selected,
        report.parsed,
        report.failed
    );
    for class in &report.classes {
        println!(
            "  class {:>3} {:<20} {:>6} objects {:>6} parsed {:>4} failed{}",
            class.class_id,
            class.type_name,
            class.objects,
            class.parsed,
            class.failed,
            if class.typed { " (typed)" } else { "" }
        );
        for failure in &class.failure_samples {
            println!("      {}: {}", failure.path_id, failure.message);
        }
    }
    if let Some(mesh) = &report.mesh {
        println!(
            "  meshes {}: {} vertices (max {}), {} indices, {} submeshes, {} skinned, {} bind poses (max {}), {} compressed, {} blend-shape",
            mesh.meshes,
            mesh.total_vertices,
            mesh.max_vertices,
            mesh.total_indices,
            mesh.submeshes,
            mesh.skinned_meshes,
            mesh.total_bind_poses,
            mesh.max_bind_poses,
            mesh.compressed_meshes,
            mesh.blend_shape_meshes
        );
    }
    if let Some(clips) = &report.animation {
        println!(
            "  clips {}: {} legacy, {} curves (rot {} pos {} scale {} float {}), {} keys, max {:.3}s / {} frames",
            clips.clips,
            clips.legacy_clips,
            clips.rotation_curves + clips.position_curves + clips.scale_curves + clips.float_curves,
            clips.rotation_curves,
            clips.position_curves,
            clips.scale_curves,
            clips.float_curves,
            clips.total_keys,
            clips.max_length_seconds,
            clips.max_frames
        );
    }
    if let Some(tree) = &report.hierarchy {
        println!(
            "  hierarchy: {} game objects, {} transforms ({} roots, depth {}), {} broken transform links, {} broken component links, {} renderers ({} mesh unresolved, {} bone unresolved, {} bone-count mismatch)",
            tree.game_objects,
            tree.transforms,
            tree.root_transforms,
            tree.max_depth,
            tree.broken_transform_links,
            tree.broken_component_links,
            tree.skinned_renderers,
            tree.renderer_mesh_unresolved,
            tree.renderer_bone_unresolved,
            tree.renderer_bone_count_mismatch
        );
    }
    Ok(())
}

fn run_assets_inspect_bundle(args: Vec<OsString>) -> Result<(), String> {
    let mut path = None;
    let mut json = false;
    let mut objects = false;
    let mut text_asset = None;
    let mut event = None;
    let mut field_execute = None;
    let mut all_functions = false;
    let mut orchestrate = false;
    let mut scan_requests = false;
    let mut talk_confirm = None;
    let mut talk_radius = None;
    let mut wander = false;
    let mut smart_walk = false;
    let mut atan_table = None;
    let mut navigation_bundle = None;
    let mut navigation_bgi = None;
    let mut ticks = 64u64;
    let mut probe = false;
    let mut inventory = false;
    let mut mbg_all = false;
    let mut audio_all = false;
    let mut audio_decode = false;
    let mut audio_out: Option<PathBuf> = None;
    let mut wav_dir: Option<PathBuf> = None;
    let mut resources_assets: Option<PathBuf> = None;
    let mut movie_inventory: Option<PathBuf> = None;
    let mut map_config_all = false;
    let mut unity_objects = false;
    let mut class_filter = None;
    let mut texture = None;
    let mut texture_id = None;
    let mut rgba_output = None;
    let mut bgi = None;
    let mut bgs = None;
    let mut field = None;
    let mut atlas_for = None;
    let mut battle_scenes = false;
    let mut battle_scripts_probe: Option<PathBuf> = None;
    let mut battle_ai_demo: Option<(String, PathBuf)> = None;
    let mut battle_sequences: Option<PathBuf> = None;
    let mut battle_model_coverage: Option<PathBuf> = None;
    let mut special_effects: Option<PathBuf> = None;
    let mut battle_dll: Option<PathBuf> = None;
    let mut battle_scene = None;
    let mut battle_rewards = false;
    let mut battle_run = false;
    let mut scripts_bundle: Option<PathBuf> = None;
    let mut battle_run_out: Option<PathBuf> = None;
    let mut battle_world_run_out: Option<PathBuf> = None;
    let mut battle_world_run = false;
    let mut all_patterns = false;
    let mut battle_seed: Option<u64> = None;
    let mut battle_weak_party = false;
    let mut battle_flee_first = false;
    let mut battle_everything_out: Option<PathBuf> = None;
    let mut world_map = false;
    let mut world_map_audit = None;
    let mut world_walk: Option<PathBuf> = None;
    let mut assemble_model_dir = None;
    let mut assemble_all_models = false;
    let mut field_models = false;
    let mut field_composition = false;
    let mut composition_png: Option<PathBuf> = None;
    let mut composition_out: Option<PathBuf> = None;
    let mut field_animation_check = false;
    let mut animation_out: Option<PathBuf> = None;
    let mut animation_camera: u8 = 0;
    let mut animation_frames: Option<(String, u32, PathBuf)> = None;
    let mut models_bundle: Option<PathBuf> = None;
    let mut clips_bundle = None;
    let mut clip_sampling_out: Option<PathBuf> = None;
    let mut clip_durations_out: Option<PathBuf> = None;
    let mut index = 2;
    while index < args.len() {
        match args[index].to_str() {
            Some("--path") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--path` requires a path".to_owned())?;
                if path.replace(PathBuf::from(value)).is_some() {
                    return Err("`--path` may only be supplied once".to_owned());
                }
            }
            Some("--text-asset") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--text-asset` requires a UTF-8 asset name".to_owned())?;
                if text_asset.replace(value.to_owned()).is_some() {
                    return Err("`--text-asset` may only be supplied once".to_owned());
                }
            }
            Some("--field-execute") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--field-execute` requires a UTF-8 asset name".to_owned())?;
                if field_execute.replace(value.to_owned()).is_some() {
                    return Err("`--field-execute` may only be supplied once".to_owned());
                }
            }
            Some("--all-functions") => all_functions = true,
            Some("--orchestrate") => orchestrate = true,
            Some("--scan-requests") => scan_requests = true,
            Some("--talk-radius") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--talk-radius` requires a byte".to_owned())?;
                talk_radius = Some(
                    value
                        .parse()
                        .map_err(|_| "`--talk-radius` requires a byte".to_owned())?,
                );
            }
            Some("--wander") => wander = true,
            Some("--smart-walk") => smart_walk = true,
            Some("--talk-confirm") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--talk-confirm` requires a mask".to_owned())?;
                talk_confirm = Some(
                    i32::from_str_radix(value.trim_start_matches("0x"), 16)
                        .map_err(|_| "`--talk-confirm` requires a hex mask".to_owned())?,
                );
            }
            Some("--navigation-bundle") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--navigation-bundle` requires a path".to_owned())?;
                if navigation_bundle.replace(PathBuf::from(value)).is_some() {
                    return Err("`--navigation-bundle` may only be supplied once".to_owned());
                }
            }
            Some("--navigation-bgi") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--navigation-bgi` requires a UTF-8 asset name".to_owned())?;
                if navigation_bgi.replace(value.to_owned()).is_some() {
                    return Err("`--navigation-bgi` may only be supplied once".to_owned());
                }
            }
            Some("--atan-table") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--atan-table` requires a path".to_owned())?;
                if atan_table.replace(PathBuf::from(value)).is_some() {
                    return Err("`--atan-table` may only be supplied once".to_owned());
                }
            }
            Some("--ticks") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--ticks` requires an integer".to_owned())?;
                ticks = value
                    .parse()
                    .map_err(|_| "`--ticks` requires an unsigned integer".to_owned())?;
            }
            Some("--event") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--event` requires a UTF-8 asset name".to_owned())?;
                if event.replace(value.to_owned()).is_some() {
                    return Err("`--event` may only be supplied once".to_owned());
                }
            }
            Some("--texture-id") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--texture-id` requires an integer".to_owned())?
                    .parse::<i64>()
                    .map_err(|error| format!("invalid texture path ID: {error}"))?;
                if texture_id.replace(value).is_some() {
                    return Err("`--texture-id` may only be supplied once".to_owned());
                }
            }
            Some("--rgba-output") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--rgba-output` requires a path".to_owned())?;
                if rgba_output.replace(PathBuf::from(value)).is_some() {
                    return Err("`--rgba-output` may only be supplied once".to_owned());
                }
            }
            Some("--texture") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--texture` requires a UTF-8 asset name".to_owned())?;
                if texture.replace(value.to_owned()).is_some() {
                    return Err("`--texture` may only be supplied once".to_owned());
                }
            }
            Some("--atlas-for") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--atlas-for` requires a UTF-8 field name".to_owned())?;
                if atlas_for.replace(value.to_owned()).is_some() {
                    return Err("`--atlas-for` may only be supplied once".to_owned());
                }
            }
            Some("--field") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--field` requires a UTF-8 field name".to_owned())?;
                if field.replace(value.to_owned()).is_some() {
                    return Err("`--field` may only be supplied once".to_owned());
                }
            }
            Some("--bgs") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--bgs` requires a UTF-8 asset name".to_owned())?;
                if bgs.replace(value.to_owned()).is_some() {
                    return Err("`--bgs` may only be supplied once".to_owned());
                }
            }
            Some("--bgi") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--bgi` requires a UTF-8 asset name".to_owned())?;
                if bgi.replace(value.to_owned()).is_some() {
                    return Err("`--bgi` may only be supplied once".to_owned());
                }
            }
            Some("--battle-scenes") => battle_scenes = true,
            Some("--battle-scripts-probe") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--battle-scripts-probe` requires an output path".to_owned())?;
                if battle_scripts_probe.replace(PathBuf::from(value)).is_some() {
                    return Err("`--battle-scripts-probe` may only be supplied once".to_owned());
                }
            }
            Some("--battle-ai-demo") => {
                let scene = args
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--battle-ai-demo` requires a scene name".to_owned())?
                    .to_owned();
                let out = args
                    .get(index + 2)
                    .ok_or_else(|| "`--battle-ai-demo` requires an output path".to_owned())?;
                index += 2;
                if battle_ai_demo
                    .replace((scene, PathBuf::from(out)))
                    .is_some()
                {
                    return Err("`--battle-ai-demo` may only be supplied once".to_owned());
                }
            }
            Some("--battle-sequences") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--battle-sequences` requires an output path".to_owned())?;
                if battle_sequences.replace(PathBuf::from(value)).is_some() {
                    return Err("`--battle-sequences` may only be supplied once".to_owned());
                }
            }
            Some("--battle-model-coverage") => {
                index += 1;
                let value = args.get(index).ok_or_else(|| {
                    "`--battle-model-coverage` requires an output path".to_owned()
                })?;
                if battle_model_coverage
                    .replace(PathBuf::from(value))
                    .is_some()
                {
                    return Err("`--battle-model-coverage` may only be supplied once".to_owned());
                }
            }
            Some("--dll") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--dll` requires an Assembly-CSharp.dll path".to_owned())?;
                if battle_dll.replace(PathBuf::from(value)).is_some() {
                    return Err("`--dll` may only be supplied once".to_owned());
                }
            }
            Some("--special-effects") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--special-effects` requires an output path".to_owned())?;
                if special_effects.replace(PathBuf::from(value)).is_some() {
                    return Err("`--special-effects` may only be supplied once".to_owned());
                }
            }
            Some("--battle-scene") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--battle-scene` requires a UTF-8 scene name".to_owned())?;
                if battle_scene.replace(value.to_owned()).is_some() {
                    return Err("`--battle-scene` may only be supplied once".to_owned());
                }
            }
            Some("--battle-rewards") => battle_rewards = true,
            Some("--battle-run") => battle_run = true,
            Some("--battle-run-all") => {
                battle_run_out = Some(PathBuf::from(value_next(
                    &args,
                    &mut index,
                    "--battle-run-all requires an output path",
                )?))
            }
            Some("--battle-world-run") => battle_world_run = true,
            Some("--battle-world-run-all") => {
                battle_world_run_out = Some(PathBuf::from(value_next(
                    &args,
                    &mut index,
                    "--battle-world-run-all requires an output path",
                )?))
            }
            Some("--all-patterns") => all_patterns = true,
            Some("--battle-seed") => {
                let value = value_next(&args, &mut index, "--battle-seed requires a u64")?;
                let value = value
                    .to_str()
                    .ok_or_else(|| "`--battle-seed` must be UTF-8".to_owned())?;
                battle_seed = Some(value.parse().map_err(|_| "`--battle-seed` must be a u64")?);
            }
            Some("--battle-weak-party") => battle_weak_party = true,
            Some("--battle-flee-first") => battle_flee_first = true,
            Some("--battle-run-everything") => {
                battle_everything_out = Some(PathBuf::from(value_next(
                    &args,
                    &mut index,
                    "--battle-run-everything requires an output directory",
                )?))
            }
            Some("--scripts-bundle") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--scripts-bundle` requires a UTF-8 bundle path".to_owned())?;
                if scripts_bundle.replace(PathBuf::from(value)).is_some() {
                    return Err("`--scripts-bundle` may only be supplied once".to_owned());
                }
            }
            Some("--json") => json = true,
            Some("--objects") => objects = true,
            Some("--probe") => probe = true,
            Some("--inventory") => inventory = true,
            Some("--mbg-all") => mbg_all = true,
            Some("--audio-all") => audio_all = true,
            Some("--audio-decode") => audio_decode = true,
            Some(flag @ ("--audio-out" | "--wav" | "--resources-assets" | "--movie-inventory")) => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{flag}` requires a path"))?;
                let slot = match flag {
                    "--audio-out" => &mut audio_out,
                    "--wav" => &mut wav_dir,
                    "--resources-assets" => &mut resources_assets,
                    _ => &mut movie_inventory,
                };
                if slot.replace(PathBuf::from(value)).is_some() {
                    return Err(format!("`{flag}` may only be supplied once"));
                }
            }
            Some("--map-config-all") => map_config_all = true,
            Some("--unity-objects") => unity_objects = true,
            Some("--world-map") => world_map = true,
            Some("--world-walk") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--world-walk` requires an output path".to_owned())?;
                world_walk = Some(PathBuf::from(value));
            }
            Some("--world-map-audit") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--world-map-audit` requires a path".to_owned())?;
                world_map_audit = Some(PathBuf::from(value));
            }
            Some("--assemble-all") => assemble_all_models = true,
            Some("--field-models") => field_models = true,
            Some("--field-composition") => field_composition = true,
            Some("--field-animation-check") => field_animation_check = true,
            Some("--animation-out") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--animation-out` requires a path".to_owned())?;
                animation_out = Some(PathBuf::from(value));
            }
            Some("--animation-camera") => {
                index += 1;
                animation_camera = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .and_then(|value| value.parse().ok())
                    .ok_or_else(|| "`--animation-camera` requires a camera number".to_owned())?;
            }
            Some("--field-animation-frames") => {
                let field = args
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--field-animation-frames` requires a field name".to_owned())?
                    .to_owned();
                let tick: u32 = args
                    .get(index + 2)
                    .and_then(|value| value.to_str())
                    .and_then(|value| value.parse().ok())
                    .ok_or_else(|| {
                        "`--field-animation-frames` requires a tick number".to_owned()
                    })?;
                let out = args.get(index + 3).ok_or_else(|| {
                    "`--field-animation-frames` requires an output PNG path".to_owned()
                })?;
                animation_frames = Some((field, tick, PathBuf::from(out)));
                index += 3;
            }
            Some(flag @ ("--png" | "--composition-out")) => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{flag}` requires a path"))?;
                let slot = if flag == "--png" {
                    &mut composition_png
                } else {
                    &mut composition_out
                };
                if slot.replace(PathBuf::from(value)).is_some() {
                    return Err(format!("`{flag}` may only be supplied once"));
                }
            }
            Some("--models") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--models` requires a model bundle path".to_owned())?;
                models_bundle = Some(PathBuf::from(value));
            }
            Some("--assemble-model") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--assemble-model` requires a model directory".to_owned())?;
                if assemble_model_dir.replace(value.to_owned()).is_some() {
                    return Err("`--assemble-model` may only be supplied once".to_owned());
                }
            }
            Some("--clips") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--clips` requires an animation bundle path".to_owned())?;
                if clips_bundle.replace(PathBuf::from(value)).is_some() {
                    return Err("`--clips` may only be supplied once".to_owned());
                }
            }
            Some("--clip-sampling") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--clip-sampling` requires an output json path".to_owned())?;
                if clip_sampling_out.replace(PathBuf::from(value)).is_some() {
                    return Err("`--clip-sampling` may only be supplied once".to_owned());
                }
            }
            Some("--clip-durations") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--clip-durations` requires an output json path".to_owned())?;
                if clip_durations_out.replace(PathBuf::from(value)).is_some() {
                    return Err("`--clip-durations` may only be supplied once".to_owned());
                }
            }
            Some("--class") => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "`--class` requires a class ID".to_owned())?
                    .parse::<u16>()
                    .map_err(|error| format!("invalid class ID: {error}"))?;
                if class_filter.replace(value).is_some() {
                    return Err("`--class` may only be supplied once".to_owned());
                }
            }
            _ => {
                return Err(format!(
                    "unrecognized argument: {}",
                    args[index].to_string_lossy()
                ));
            }
        }
        index += 1;
    }
    let path = path.ok_or_else(|| "missing required `--path <path>`".to_owned())?;
    if audio_decode {
        return run_audio_decode(&AudioDecodeRequest {
            path: &path,
            out: audio_out.as_deref(),
            wav_dir: wav_dir.as_deref(),
            resources_assets: resources_assets.as_deref(),
            movie_inventory: movie_inventory.as_deref(),
            json,
        });
    }
    let battle_seed = battle_seed.unwrap_or(0x5EED);
    if let Some(out_dir) = battle_everything_out.as_deref() {
        return run_battle_run_everything(&path, scripts_bundle.as_deref(), out_dir, battle_seed);
    }
    if let Some(out) = battle_run_out.as_deref() {
        return run_battle_run_all(
            &path,
            scripts_bundle.as_deref(),
            out,
            all_patterns,
            battle_seed,
            battle_weak_party,
            battle_flee_first,
        );
    }
    if let Some(out) = battle_world_run_out.as_deref() {
        return run_battle_world_run_all(
            &path,
            scripts_bundle.as_deref(),
            out,
            all_patterns,
            battle_seed,
            battle_weak_party,
            battle_flee_first,
        );
    }
    if battle_scenes || battle_scene.is_some() {
        if battle_world_run {
            return run_battle_world_run(
                &path,
                battle_scene.as_deref(),
                scripts_bundle.as_deref(),
                json,
            );
        }
        if battle_run {
            return run_battle_run(&path, battle_scene.as_deref(), scripts_bundle.as_deref());
        }
        return run_battle_scene_report(&path, battle_scene.as_deref(), battle_rewards);
    }
    if let Some(out) = battle_sequences.as_deref() {
        return run_battle_sequences(&path, out);
    }
    if let Some(out) = special_effects {
        return tantalus_tools::special_effects_cli::run_special_effects(
            &path,
            &tantalus_tools::special_effects_cli::SpecialEffectsRequest {
                dll: battle_dll,
                resources_assets,
                out,
            },
        );
    }
    if let Some(out) = battle_model_coverage {
        return tantalus_tools::battle_models_cli::run_battle_model_coverage(
            &path,
            &tantalus_tools::battle_models_cli::BattleModelsRequest {
                models: models_bundle,
                clips: clips_bundle,
                dll: battle_dll,
                resources_assets,
                out,
            },
        );
    }
    if field_composition {
        return run_field_composition(&path, composition_out, composition_png);
    }
    if field_animation_check {
        return tantalus_tools::field_animation_cli::run_field_animation_check(
            &path,
            animation_out,
        );
    }
    if let Some((field, tick, out)) = &animation_frames {
        return tantalus_tools::field_animation_cli::run_field_animation_frames(
            &path,
            field,
            *tick,
            animation_camera,
            out,
        );
    }
    if let Some(out) = world_walk.as_deref() {
        return tantalus_tools::world_walk_cli::run_world_walk(&path, out, json);
    }
    if let Some((scene, out)) = battle_ai_demo.as_ref() {
        return tantalus_tools::battle_ai_demo::run_demo(&path, scene, out, json);
    }
    let bundle = tantalus_assets::pc::unity_raw::UnityRawBundle::open(&path)
        .map_err(|error| error.to_string())?;
    if world_map {
        return run_world_map(&bundle, &path, json, world_map_audit.as_deref());
    }
    if let Some(out) = battle_scripts_probe.as_deref() {
        return run_battle_scripts_probe(&bundle, &path, out, json);
    }
    if let Some(name) = event {
        let bytes = bundle
            .read_entry(0, 300 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let serialized =
            tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
                .map_err(|error| error.to_string())?;
        let mut candidates = Vec::new();
        for (index, object) in serialized.report().objects.iter().enumerate() {
            if object.class_id != 49 {
                continue;
            }
            let asset = tantalus_assets::pc::text_asset::TextAsset::parse(
                serialized
                    .object_data(index)
                    .map_err(|error| error.to_string())?,
                &name,
            )
            .map_err(|error| error.to_string())?;
            if asset.name != name {
                continue;
            }
            let binary = tantalus_legacy::EventBinary::parse(&asset.script)
                .map_err(|error| format!("{}: {error}", object.path_id))?;
            let mut digest = Sha256::new();
            let mut script_bytes = 0_usize;
            for entry in &binary.entries {
                let script = binary
                    .entry_bytes(entry.index)
                    .expect("validated event entry");
                script_bytes += script.len();
                digest.update(script);
            }
            let mut decoded_entries = 0_usize;
            let mut decoded_functions = 0_usize;
            let function_count = binary
                .entries
                .iter()
                .map(|entry| {
                    binary
                        .entry_bytes(entry.index)
                        .and_then(|bytes| bytes.get(1))
                        .copied()
                        .unwrap_or(0) as usize
                })
                .sum::<usize>();
            let mut decode_errors = Vec::new();
            for entry in &binary.entries {
                match binary.decode_entry_with_diagnostics(entry.index) {
                    Ok(decoded) => {
                        decoded_functions += decoded.functions.len();
                        if decoded.errors.is_empty() {
                            decoded_entries += 1;
                        }
                        for failure in decoded.errors {
                            let error = failure.error;
                            decode_errors.push(serde_json::json!({
                                "entry": entry.index,
                                "function": failure.function,
                                "tag": failure.tag,
                                "kind": format!("{:?}", error.kind),
                                "offset": error.offset,
                                "opcode": error.opcode,
                                "message": error.message,
                            }));
                        }
                    }
                    Err(error) => decode_errors.push(serde_json::json!({
                        "entry": entry.index,
                        "kind": format!("{:?}", error.kind),
                        "offset": error.offset,
                        "opcode": error.opcode,
                        "message": error.message,
                    })),
                }
            }
            let probe_report = if probe {
                let mut probed = 0_usize;
                let mut empty = 0_usize;
                let mut completed = 0_usize;
                let mut waiting = 0_usize;
                let mut failures: std::collections::BTreeMap<String, (usize, String)> =
                    std::collections::BTreeMap::new();
                let mut events: std::collections::BTreeMap<String, usize> =
                    std::collections::BTreeMap::new();
                for entry in &binary.entries {
                    let Ok(decoded) = binary.decode_entry_with_diagnostics(entry.index) else {
                        continue;
                    };
                    for function in &decoded.functions {
                        if function.program.instructions.is_empty() {
                            empty += 1;
                            continue;
                        }
                        probed += 1;
                        let outcome = tantalus_legacy::probe_program(
                            &function.program,
                            tantalus_contracts::EntityId::new(entry.index as u64),
                            64,
                        );
                        for (name, count) in outcome.events {
                            *events.entry(name).or_insert(0) += count;
                        }
                        match outcome.error {
                            Some(failure) => {
                                let key = format!(
                                    "{:?} opcode {:?}: {}",
                                    failure.kind, failure.opcode, failure.message
                                );
                                let slot = failures.entry(key).or_insert_with(|| {
                                    (
                                        0,
                                        format!(
                                            "entry {} tag {} pc {}",
                                            entry.index, function.tag, failure.offset
                                        ),
                                    )
                                });
                                slot.0 += 1;
                            }
                            None if outcome.status == tantalus_legacy::VmStatus::Completed => {
                                completed += 1;
                            }
                            None => waiting += 1,
                        }
                    }
                }
                Some(serde_json::json!({
                    "scope": "zero-state dry run: permissive host, reads return zero, waits finish; not a gameplay claim",
                    "max_ticks": 64,
                    "functions_probed": probed,
                    "empty_functions": empty,
                    "completed": completed,
                    "still_waiting": waiting,
                    "runtime_errors": failures
                        .iter()
                        .map(|(message, (count, example))| serde_json::json!({
                            "message": message,
                            "count": count,
                            "first_at": example,
                        }))
                        .collect::<Vec<_>>(),
                    "events": events,
                }))
            } else {
                None
            };
            let first_decode_error = decode_errors.first().cloned();
            candidates.push(serde_json::json!({
                "path_id": object.path_id,
                "asset_bytes": asset.script.len(),
                "version": binary.version,
                "entries": binary.entries.len(),
                "functions": function_count,
                "decoded_entries": decoded_entries,
                "decoded_functions": decoded_functions,
                "first_decode_error": first_decode_error,
                "decode_errors": decode_errors,
                "script_bytes": script_bytes,
                "script_sha256": format!("{:x}", digest.finalize()),
                "probe": probe_report,
            }));
        }
        if candidates.is_empty() {
            return Err(format!("event asset `{name}` was not found"));
        }
        let report = serde_json::json!({"name": name, "candidates": candidates});
        if json {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else {
            println!("{name}: {} event binaries", candidates.len());
        }
        return Ok(());
    }
    if scan_requests {
        let bytes = bundle
            .read_entry(0, 300 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let serialized =
            tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
                .map_err(|error| error.to_string())?;
        let mut candidates = Vec::new();
        let mut scanned = 0usize;
        for (index, object) in serialized.report().objects.iter().enumerate() {
            if object.class_id != 49 {
                continue;
            }
            let data = match serialized.object_data(index) {
                Ok(data) => data,
                Err(_) => continue,
            };
            let name_probe = String::new();
            let asset = match tantalus_assets::pc::text_asset::TextAsset::parse(data, &name_probe) {
                Ok(asset) => asset,
                Err(_) => continue,
            };
            if !asset.name.ends_with(".eb") {
                continue;
            }
            scanned += 1;
            let Ok(binary) = tantalus_legacy::EventBinary::parse(&asset.script) else {
                continue;
            };
            for entry in &binary.entries {
                let Ok(decoded) = binary.decode_entry_with_diagnostics(entry.index) else {
                    continue;
                };
                let has_request = decoded.functions.iter().any(|function| {
                    function.program.instructions.values().any(|instruction| {
                        matches!(
                            instruction.kind,
                            tantalus_legacy::InstructionKind::RequestScriptAsync { .. }
                                | tantalus_legacy::InstructionKind::RequestScriptSync { .. }
                                | tantalus_legacy::InstructionKind::RequestCallerAsync { .. }
                                | tantalus_legacy::InstructionKind::RequestCallerSync { .. }
                                | tantalus_legacy::InstructionKind::RunSharedScript { .. }
                        )
                    })
                });
                if has_request {
                    candidates.push(asset.name.clone());
                    break;
                }
            }
        }
        let report = serde_json::json!({
            "bundle": path.file_name().map(|n| n.to_string_lossy().into_owned()),
            "event_assets_scanned": scanned,
            "requesting_archives": candidates,
        });
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return Ok(());
    }
    if let Some(name) = field_execute {
        let bytes = bundle
            .read_entry(0, 300 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let serialized =
            tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
                .map_err(|error| error.to_string())?;
        let mut found = None;
        for (index, object) in serialized.report().objects.iter().enumerate() {
            if object.class_id != 49 {
                continue;
            }
            let asset = tantalus_assets::pc::text_asset::TextAsset::parse(
                serialized
                    .object_data(index)
                    .map_err(|error| error.to_string())?,
                &name,
            )
            .map_err(|error| error.to_string())?;
            if asset.name == name {
                found = Some(asset.script);
                break;
            }
        }
        let script = found.ok_or_else(|| format!("event asset `{name}` was not found"))?;
        let binary =
            tantalus_legacy::EventBinary::parse(&script).map_err(|error| error.to_string())?;
        // By default each entry's first function runs as one scene entry;
        // --all-functions registers every decoded function as its own VM for
        // coverage. The reference's function-call orchestration (which
        // function an entry runs first, how REQ targets tags) is host-owned
        // and not modeled by either mode.
        let mut director = tantalus_game::FieldDirector::new(tantalus_game::FieldWorld::new());
        if let Some(table_path) = atan_table {
            let table = std::fs::read(&table_path)
                .map_err(|error| format!("cannot read {}: {error}", table_path.display()))?;
            director.world_mut().provide_atan_table(table);
        }
        if let (Some(bundle_path), Some(bgi_name)) = (&navigation_bundle, &navigation_bgi) {
            let nav_bundle = tantalus_assets::pc::unity_raw::UnityRawBundle::open(bundle_path)
                .map_err(|error| error.to_string())?;
            let nav_bytes = nav_bundle
                .read_entry(0, 300 * 1024 * 1024)
                .map_err(|error| error.to_string())?;
            let nav_serialized = tantalus_assets::pc::serialized::SerializedFile::parse(
                &nav_bytes,
                &bundle_path.to_string_lossy(),
            )
            .map_err(|error| error.to_string())?;
            let mut nav_asset = None;
            for (index, object) in nav_serialized.report().objects.iter().enumerate() {
                if object.class_id != 49 {
                    continue;
                }
                let asset = tantalus_assets::pc::text_asset::TextAsset::parse(
                    nav_serialized
                        .object_data(index)
                        .map_err(|error| error.to_string())?,
                    bgi_name,
                )
                .map_err(|error| error.to_string())?;
                if asset.name == *bgi_name {
                    nav_asset = Some(asset.script);
                    break;
                }
            }
            let script =
                nav_asset.ok_or_else(|| format!("BGI asset `{bgi_name}` was not found"))?;
            let bgi = tantalus_assets::pc::bgi::BgiFile::parse(&script, bgi_name)
                .map_err(|error| error.to_string())?;
            let mesh = tantalus_assets::pc::field::bgi_to_navigation_mesh(&bgi)
                .map_err(|error| error.to_string())?;
            director.world_mut().provide_navigation(mesh);
        }
        let mut entry_tags = Vec::new();
        let mut skipped = Vec::new();
        // Orchestration pool: every decoded function per entry, spawned on
        // demand by script requests (uid == entry index convention).
        let mut pool: Vec<Vec<(u16, tantalus_legacy::EventProgram)>> = Vec::new();
        for entry in &binary.entries {
            let decoded = binary
                .decode_entry_with_diagnostics(entry.index)
                .map_err(|error| error.to_string())?;
            pool.push(
                decoded
                    .functions
                    .iter()
                    .filter(|function| !function.program.instructions.is_empty())
                    .map(|function| (function.tag, function.program.clone()))
                    .collect(),
            );
        }
        let register = |director: &mut tantalus_game::FieldDirector,
                        entry: usize,
                        tag: Option<u16>,
                        ordinal: usize|
         -> Result<Option<(tantalus_contracts::EntityId, u16)>, String> {
            let functions = &pool[entry];
            let selected = match tag {
                Some(tag) => functions.iter().position(|(t, _)| *t == tag),
                None => {
                    if functions.is_empty() {
                        None
                    } else {
                        Some(ordinal.min(functions.len() - 1))
                    }
                }
            };
            let Some(position) = selected else {
                return Ok(None);
            };
            let (function_tag, program) = &functions[position];
            let entity = tantalus_contracts::EntityId::new(entry as u64 * 256 + position as u64);
            director
                .add_entry_program(entity, program.clone())
                .map_err(|error| error.to_string())?;
            director.world_mut().add_actor(entity, [0.0; 3]);
            Ok(Some((entity, *function_tag)))
        };
        // Objects exist at field start: every entry has a base actor keyed by
        // its index (the uid convention), so member reads against objects an
        // entry's init has not run yet still resolve to real state.
        for entry in 0..pool.len() {
            let base = tantalus_contracts::EntityId::new(entry as u64);
            director.world_mut().add_actor(base, [0.0; 3]);
            director.world_mut().map_uid(entry as i32, base);
        }
        // Slots 251..255 are the always-present party member objects.
        for slot in 251u64..256 {
            let entity = tantalus_contracts::EntityId::new(slot);
            director.world_mut().add_actor(entity, [0.0; 3]);
            director.world_mut().map_uid(slot as i32, entity);
        }
        if orchestrate {
            for (entry, functions) in pool.iter().enumerate() {
                if functions.is_empty() {
                    skipped.push(entry);
                    continue;
                }
                // Initial VMs run each entry's tag-0 function (fallback: the
                // first function), keyed by the entry index as the uid.
                if let Some((_, function_tag)) = register(&mut director, entry, Some(0), 0)? {
                    entry_tags.push((entry, function_tag));
                }
            }
        } else {
            for (entry, functions) in pool.iter().enumerate() {
                if all_functions {
                    for ordinal in 0..functions.len() {
                        if let Some((_, function_tag)) =
                            register(&mut director, entry, None, ordinal)?
                        {
                            entry_tags.push((entry, function_tag));
                        }
                    }
                } else if !functions.is_empty() {
                    if let Some((_, function_tag)) = register(&mut director, entry, None, 0)? {
                        entry_tags.push((entry, function_tag));
                    }
                } else {
                    skipped.push(entry);
                }
            }
        }
        if let Some(mask) = talk_confirm {
            // Demo mode: entity 0 acts as the player holding confirm, so
            // talk triggers fire wherever an object set a talk radius.
            director.set_player(tantalus_contracts::EntityId::new(0));
            director.set_confirm_mask(Some(mask));
            director.set_input_mask(mask);
        }
        // Demo direction mapping (synthetic bits, not the pad layout):
        // 0x01 up, 0x02 down, 0x04 left, 0x08 right; confirm bit preserved.
        let confirm_only = talk_confirm.unwrap_or(0);
        if wander || smart_walk {
            director.set_walk_bits(Some((0x01, 0x02, 0x04, 0x08)));
        }
        if let Some(radius) = talk_radius {
            // Demo aid: the real engine assigns talk radii from field init
            // data we do not import yet; assign uniformly so the tag-3
            // dispatch path is exercisable on real archives.
            for entry in 0..pool.len() {
                director
                    .world_mut()
                    .set_talk_radius(tantalus_contracts::EntityId::new(entry as u64), radius);
            }
        }
        let mut operations = 0u64;
        let mut error = None;
        let mut ticks_run = 0u64;
        let mut spawned: Vec<(tantalus_contracts::EntityId, u64)> = Vec::new();
        let mut unspawned_requests = 0usize;
        let mut regions_visited: std::collections::BTreeSet<u64> =
            std::collections::BTreeSet::new();
        // Smart walk: stand by default; when user control is on and the
        // scene has not progressed for a while, walk one leg, then re-
        // evaluate. Naive constant walking disrupts scene choreography.
        let mut leg_remaining = 0u64;
        let mut leg_phase = 0usize;
        let mut facing_polls_seen = 0u64;
        for tick in 0..ticks {
            if wander {
                // 64 ticks per leg: 1024-unit square covering the field.
                let phase = (tick / 64) % 4;
                // The Alexandria fields live in negative coordinates; tour
                // left/up first.
                let direction = [0x04u32, 0x01, 0x08, 0x02][phase as usize];
                director.set_input_mask(confirm_only | i32::from(direction as u8));
            }
            if smart_walk {
                if tick == 0 {
                    director.set_input_mask(confirm_only);
                }
                // Walk only on a positively detected facing gate: user
                // control on and the scene is polling the player actor's
                // facing (member 3).
                let facing_polled = director.world().player_facing_polls > facing_polls_seen;
                facing_polls_seen = director.world().player_facing_polls;
                if smart_walk
                    && director.world().user_control
                    && facing_polled
                    && leg_remaining == 0
                {
                    leg_remaining = 64;
                    leg_phase = (leg_phase + 1) % 4;
                }
                if leg_remaining > 0 {
                    // Alternate opposite directions each tick: facing swings
                    // through its full range (satisfying facing gates) while
                    // net displacement stays near zero, so scene anchors
                    // hold. 0x08 east / 0x04 west, then 0x01/0x02 on Z.
                    let pair = [[0x08u8, 0x04u8], [0x01, 0x02]][leg_phase % 2];
                    let direction = pair[(leg_remaining % 2) as usize];
                    director.set_input_mask(confirm_only | i32::from(direction));
                    leg_remaining -= 1;
                    if leg_remaining == 0 {
                        director.set_input_mask(confirm_only);
                    }
                }
            }
            match director.step() {
                Ok(step) => {
                    operations += step.operations;
                    ticks_run += 1;
                    for region in &step.region_occupancy {
                        regions_visited.insert(region.get());
                    }
                    if orchestrate {
                        for request in &step.requests {
                            let target = request.target.get() as usize;
                            if target >= pool.len() {
                                unspawned_requests += 1;
                                continue;
                            }
                            let tag = u16::from(request.tag);
                            let tag = if pool[target].iter().any(|(t, _)| *t == tag) {
                                Some(tag)
                            } else {
                                None
                            };
                            match register(&mut director, target, tag, 0) {
                                Ok(Some((entity, function_tag))) => {
                                    entry_tags.push((target, function_tag));
                                    spawned.push((entity, target as u64));
                                }
                                Ok(None) => unspawned_requests += 1,
                                Err(_) => unspawned_requests += 1,
                            }
                        }
                        // Talk dispatch: the reference calls the object's
                        // function tag 3 (EventEngine.IsActuallyTalkable,
                        // tagId = 3) when the player confirms in range.
                        for trigger in &step.talk_triggers {
                            let target = trigger.get() as usize;
                            if target >= pool.len() {
                                continue;
                            }
                            if let Ok(Some((entity, function_tag))) =
                                register(&mut director, target, Some(3), 0)
                            {
                                entry_tags.push((target, function_tag));
                                spawned.push((entity, target as u64));
                            }
                        }
                    }
                }
                Err(failure) => {
                    error = Some(format!("{failure}"));
                    break;
                }
            }
            if orchestrate {
                let states = director.entry_states();
                let mut completed_bases = Vec::new();
                spawned.retain(|(entity, base)| {
                    let done = states.iter().any(|(state_entity, _, status, _)| {
                        state_entity == entity && *status == tantalus_legacy::VmStatus::Completed
                    });
                    if done {
                        completed_bases.push(*base);
                    }
                    !done
                });
                for base in completed_bases {
                    director
                        .world_mut()
                        .complete_request(tantalus_contracts::EntityId::new(base));
                }
            }
        }
        let states = director
            .entry_states()
            .into_iter()
            .map(|(entity, pc, status, wait)| {
                let position = director
                    .world()
                    .actors
                    .get(&entity)
                    .map(|state| state.position);
                serde_json::json!({
                    "entry": entity.get(),
                    "pc": pc,
                    "status": format!("{status:?}"),
                    "wait": wait.map(|wait| format!("{wait:?}")),
                    "position": position,
                })
            })
            .collect::<Vec<_>>();
        let completed = states
            .iter()
            .filter(|state| {
                state
                    .get("status")
                    .and_then(|status| status.as_str())
                    .map(|status| status == "Completed")
                    .unwrap_or(false)
            })
            .count();
        let report = serde_json::json!({
            "name": name,
            "ticks_requested": ticks,
            "ticks_run": ticks_run,
            "operations": operations,
            "entries": entry_tags
                .iter()
                .map(|(entry, tag)| serde_json::json!({"entry": entry, "tag": tag}))
                .collect::<Vec<_>>(),
            "skipped_empty_entries": skipped,
            "orchestrated": orchestrate,
            "smart_walk": smart_walk,
            "walk_legs": leg_phase + 1,
            "regions_visited": regions_visited.len(),
            "regions_defined": director.world().regions.len(),
            "spawned_functions": spawned.len(),
            "unspawned_requests": unspawned_requests,
            "completed_entries": completed,
            "final_states": states,
            "error": error,
        });
        if json {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else {
            println!(
                "{name}: {} entries, {operations} operations in {ticks_run} ticks, {completed} completed",
                entry_tags.len()
            );
        }
        return Ok(());
    }
    if audio_all {
        return run_audio_all(&path, json);
    }
    if mbg_all || map_config_all {
        return run_parse_all(&bundle, &path, mbg_all, map_config_all);
    }
    if field_models {
        return tantalus_tools::field_models_cli::run_field_models(
            &bundle,
            &path,
            &tantalus_tools::field_models_cli::FieldModelsRequest {
                models: models_bundle,
                clips: clips_bundle,
                json,
            },
        );
    }
    if let Some(out) = clip_durations_out.as_deref() {
        let clips_path = clips_bundle
            .as_deref()
            .ok_or_else(|| "`--clip-durations` requires `--clips <p0data5.bin>`".to_owned())?;
        return tantalus_tools::model_cli::run_clip_durations(clips_path, out);
    }
    if assemble_model_dir.is_some() || assemble_all_models || clip_sampling_out.is_some() {
        return tantalus_tools::model_cli::run_model_request(
            &bundle,
            &path,
            &tantalus_tools::model_cli::ModelRequest {
                model: assemble_model_dir,
                all: assemble_all_models || clip_sampling_out.is_some(),
                clips: clips_bundle,
                clip_sampling_out,
                json,
            },
        );
    }
    if unity_objects {
        return run_unity_objects(&bundle, &path, class_filter, json);
    }
    if class_filter.is_some() {
        return Err("`--class` requires `--unity-objects`".to_owned());
    }
    if inventory {
        let bytes = bundle
            .read_entry(0, 600 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let serialized =
            tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
                .map_err(|error| error.to_string())?;
        let report = serialized.report();
        let classes: std::collections::BTreeMap<i64, u32> = report
            .objects
            .iter()
            .map(|object| (object.path_id, object.class_id as u32))
            .collect();
        let mut class_counts: std::collections::BTreeMap<u32, usize> =
            std::collections::BTreeMap::new();
        for object in &report.objects {
            *class_counts.entry(object.class_id as u32).or_insert(0) += 1;
        }
        let (container_index, _) = report
            .objects
            .iter()
            .enumerate()
            .find(|(_, object)| object.class_id == 142)
            .ok_or_else(|| "AssetBundle metadata object is missing".to_owned())?;
        let metadata = tantalus_assets::pc::asset_bundle::AssetBundleMetadata::parse(
            serialized
                .object_data(container_index)
                .map_err(|error| error.to_string())?,
            &path.to_string_lossy(),
        )
        .map_err(|error| error.to_string())?;
        let entries: Vec<_> = metadata
            .container
            .iter()
            .map(|(asset_path, info)| {
                serde_json::json!({
                    "path": asset_path,
                    "path_id": info.path_id,
                    "class_id": classes.get(&info.path_id),
                })
            })
            .collect();
        let inventory_report = serde_json::json!({
            "bundle": path.file_name().map(|name| name.to_string_lossy().into_owned()),
            "object_count": report.objects.len(),
            "class_counts": class_counts,
            "container_entries": entries.len(),
            "container": entries,
        });
        println!("{}", serde_json::to_string(&inventory_report).unwrap());
        return Ok(());
    }
    if let Some(field) = atlas_for {
        let bytes = bundle
            .read_entry(0, 300 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let serialized =
            tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
                .map_err(|error| error.to_string())?;
        let (index, _) = serialized
            .report()
            .objects
            .iter()
            .enumerate()
            .find(|(_, object)| object.class_id == 142)
            .ok_or_else(|| "AssetBundle metadata object is missing".to_owned())?;
        let metadata = tantalus_assets::pc::asset_bundle::AssetBundleMetadata::parse(
            serialized
                .object_data(index)
                .map_err(|error| error.to_string())?,
            &field,
        )
        .map_err(|error| error.to_string())?;
        let info = metadata
            .atlas_for(&field)
            .ok_or_else(|| format!("no atlas is registered for `{field}`"))?;
        let report = serde_json::json!({
            "field": field,
            "path_id": info.path_id,
            "file_id": info.file_id,
            "preload_index": info.preload_index,
            "preload_size": info.preload_size,
        });
        if json {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else {
            println!("{field}: atlas path ID {}", info.path_id);
        }
        return Ok(());
    }
    if let Some(field) = field {
        let loaded = tantalus_assets::pc::bundled_field::BundledField::load(&path, &field)
            .map_err(|error| error.to_string())?;
        let mapped = &loaded.scene;
        let navigation = tantalus_game::navigation::FieldNavigation::new(&mapped.navigation)
            .map_err(|error| error.to_string())?;
        let projected_anchor = navigation
            .project(mapped.actor_anchor)
            .map_err(|error| error.to_string())?;
        let eastward_probe = navigation
            .move_by(projected_anchor.position, [16, 0])
            .map_err(|error| error.to_string())?;
        let mut active_sprites = 0_usize;
        let mut viewport_sprites = 0_usize;
        let mut bounds = [i32::MAX, i32::MIN, i32::MAX, i32::MIN];
        for overlay in &mapped.overlays {
            for sprite in &overlay.sprites {
                if sprite.active && overlay.camera_index == mapped.camera_index as u8 {
                    active_sprites += 1;
                    bounds[0] = bounds[0].min(sprite.pixel_offset[0]);
                    bounds[1] = bounds[1].max(sprite.pixel_offset[0]);
                    bounds[2] = bounds[2].min(sprite.pixel_offset[1]);
                    bounds[3] = bounds[3].max(sprite.pixel_offset[1]);
                    let width = sprite.pixel_size[0] as i32;
                    let height = sprite.pixel_size[1] as i32;
                    if sprite.pixel_offset[0] < 160
                        && sprite.pixel_offset[0] + width > -160
                        && -sprite.pixel_offset[1] > -120
                        && -sprite.pixel_offset[1] - height < 120
                    {
                        viewport_sprites += 1;
                    }
                }
            }
        }
        let report = serde_json::json!({
            "name": field,
            "scene_offset": loaded.scene_offset,
            "active_sprites": active_sprites,
            "viewport_sprites": viewport_sprites,
            "active_offset_bounds": (active_sprites > 0).then_some(bounds),
            "atlas": loaded.atlas_id.as_str(),
            "atlas_path_id": loaded.atlas_path_id,
            "camera_projection": mapped.camera.projection,
            "camera_size": mapped.camera.size,
            "camera_center_offset": mapped.camera.center_offset,
            "camera_viewport_x": mapped.camera.viewport_x,
            "camera_viewport_y": mapped.camera.viewport_y,
            "camera_translation": [mapped.camera.translation.x, mapped.camera.translation.y, mapped.camera.translation.z],
            "actor_anchor": [mapped.actor_anchor.x, mapped.actor_anchor.y, mapped.actor_anchor.z],
            "projected_actor_anchor": [projected_anchor.position.x, projected_anchor.position.y, projected_anchor.position.z],
            "projected_triangle": projected_anchor.triangle_index,
            "eastward_probe": [eastward_probe.position.x, eastward_probe.position.y, eastward_probe.position.z],
            "eastward_probe_triangle": eastward_probe.triangle_index,
            "navigation_triangles": mapped.navigation.triangles.len(),
            "overlays": mapped.overlays.len(),
            "sprites": mapped.overlays.iter().map(|overlay| overlay.sprites.len()).sum::<usize>(),
        });
        if json {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else {
            println!(
                "{}: {} overlays, {} navigation triangles, atlas path ID {}, actor anchor {:?}",
                field,
                mapped.overlays.len(),
                mapped.navigation.triangles.len(),
                loaded.atlas_path_id,
                mapped.actor_anchor
            );
        }
        return Ok(());
    }
    if let Some(name) = bgs {
        let bytes = bundle
            .read_entry(0, 300 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let serialized =
            tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
                .map_err(|error| error.to_string())?;
        let mut selected = None;
        for (index, object) in serialized.report().objects.iter().enumerate() {
            if object.class_id != 49 {
                continue;
            }
            let asset = tantalus_assets::pc::text_asset::TextAsset::parse(
                serialized
                    .object_data(index)
                    .map_err(|error| error.to_string())?,
                &name,
            )
            .map_err(|error| error.to_string())?;
            if asset.name == name {
                selected = Some(asset);
                break;
            }
        }
        let asset = selected.ok_or_else(|| format!("BGS asset `{name}` was not found"))?;
        let scene = tantalus_assets::pc::bgs::BgsFile::parse(&asset.script, &name)
            .map_err(|error| error.to_string())?;
        let report = serde_json::json!({
            "name": asset.name,
            "scene_length": scene.scene_length,
            "depth_bit_shift": scene.depth_bit_shift,
            "cameras": scene.cameras.len(),
            "overlays": scene.overlays.len(),
            "sprites": scene.overlays.iter().map(|overlay| overlay.sprites.len()).sum::<usize>(),
            "lights": scene.lights.len(),
            "animations": scene.animations.len(),
            "animation_frames": scene.animations.iter().map(|animation| animation.frames.len()).sum::<usize>(),
            "first_projection": scene.cameras.first().map(|camera| camera.projection),
        });
        if json {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else {
            println!(
                "{}: {} bytes, {} cameras, {} overlays, {} sprites",
                asset.name,
                scene.scene_length,
                scene.cameras.len(),
                scene.overlays.len(),
                scene
                    .overlays
                    .iter()
                    .map(|overlay| overlay.sprites.len())
                    .sum::<usize>()
            );
        }
        return Ok(());
    }
    if let Some(name) = bgi {
        let bytes = bundle
            .read_entry(0, 300 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let serialized =
            tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
                .map_err(|error| error.to_string())?;
        let mut selected = None;
        for (index, object) in serialized.report().objects.iter().enumerate() {
            if object.class_id != 49 {
                continue;
            }
            let asset = tantalus_assets::pc::text_asset::TextAsset::parse(
                serialized
                    .object_data(index)
                    .map_err(|error| error.to_string())?,
                &name,
            )
            .map_err(|error| error.to_string())?;
            if asset.name == name {
                selected = Some(asset);
                break;
            }
        }
        let asset = selected.ok_or_else(|| format!("BGI asset `{name}` was not found"))?;
        let navigation = tantalus_assets::pc::bgi::BgiFile::parse(&asset.script, &name)
            .map_err(|error| error.to_string())?;
        let report = serde_json::json!({
            "name": asset.name,
            "triangles": navigation.triangles.len(),
            "vertices": navigation.vertices.len(),
            "edges": navigation.edges.len(),
            "floors": navigation.floors.len(),
            "normals": navigation.normals.len(),
            "animations": navigation.animations.len(),
            "active_floor": navigation.active_floor,
            "active_triangle": navigation.active_triangle,
        });
        if json {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else {
            println!(
                "{}: {} triangles, {} vertices, {} floors",
                asset.name,
                navigation.triangles.len(),
                navigation.vertices.len(),
                navigation.floors.len()
            );
        }
        return Ok(());
    }
    if let Some(name) = text_asset {
        let bytes = bundle
            .read_entry(0, 300 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let serialized =
            tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
                .map_err(|error| error.to_string())?;
        let mut selected = None;
        let mut failures = Vec::new();
        for (index, object) in serialized.report().objects.iter().enumerate() {
            if object.class_id != 49 {
                continue;
            }
            let bytes = serialized
                .object_data(index)
                .map_err(|error| error.to_string())?;
            match tantalus_assets::pc::text_asset::TextAsset::parse(bytes, &name) {
                Ok(asset) if asset.name == name => {
                    selected = Some(index);
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    failures.push(format!("path {}: {error}", object.path_id));
                }
            }
        }
        let index = selected.ok_or_else(|| {
            format!(
                "TextAsset `{name}` was not found; decode failures include: {}",
                failures.into_iter().take(3).collect::<Vec<_>>().join("; ")
            )
        })?;
        let asset = tantalus_assets::pc::text_asset::TextAsset::parse(
            serialized
                .object_data(index)
                .map_err(|error| error.to_string())?,
            &name,
        )
        .map_err(|error| error.to_string())?;
        let report = serde_json::json!({
            "name": asset.name,
            "path_name": asset.path_name,
            "script_bytes": asset.script.len(),
            "script_sha256": format!("{:x}", Sha256::digest(&asset.script)),
        });
        if json {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else {
            println!(
                "{}: {} payload bytes, SHA-256 {}",
                report["name"].as_str().unwrap(),
                report["script_bytes"].as_u64().unwrap(),
                report["script_sha256"].as_str().unwrap()
            );
        }
        return Ok(());
    }
    if texture.is_some() || texture_id.is_some() {
        let name = texture.clone().unwrap_or_else(|| "texture".to_owned());
        let bytes = bundle
            .read_entry(0, 300 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let serialized =
            tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
                .map_err(|error| error.to_string())?;
        let mut selected = None;
        for (index, object) in serialized.report().objects.iter().enumerate() {
            if object.class_id != 28 || texture_id.is_some_and(|id| id != object.path_id) {
                continue;
            }
            let candidate = tantalus_assets::pc::texture::Texture2D::parse(
                serialized
                    .object_data(index)
                    .map_err(|error| error.to_string())?,
                &name,
            )
            .map_err(|error| error.to_string())?;
            if texture.as_deref().is_none_or(|name| candidate.name == name) {
                selected = Some((object.path_id, candidate));
                break;
            }
        }
        let (path_id, texture) =
            selected.ok_or_else(|| format!("Texture2D `{name}` was not found"))?;
        let rgba = texture.rgba_bytes().map_err(|error| error.to_string())?;
        if let Some(output) = rgba_output {
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
            }
            std::fs::write(&output, &rgba)
                .map_err(|error| format!("could not write {}: {error}", output.display()))?;
        }
        let report = serde_json::json!({
            "name": texture.name,
            "path_id": path_id,
            "width": texture.width,
            "height": texture.height,
            "format": texture.format,
            "image_bytes": texture.image_data.len(),
            "dxt5_sha256": format!("{:x}", Sha256::digest(&texture.image_data)),
            "rgba_sha256": format!("{:x}", Sha256::digest(&rgba)),
        });
        if json {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else {
            println!(
                "{}: {}x{}, format {}, {} bytes",
                texture.name,
                texture.width,
                texture.height,
                texture.format,
                texture.image_data.len()
            );
        }
        return Ok(());
    }
    if objects {
        let bytes = bundle
            .read_entry(0, 300 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let serialized =
            tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
                .map_err(|error| error.to_string())?;
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(serialized.report())
                    .map_err(|error| format!("could not serialize object report: {error}"))?
            );
        } else {
            let mut classes = std::collections::BTreeMap::new();
            for object in &serialized.report().objects {
                *classes.entry(object.class_id).or_insert(0) += 1;
            }
            println!(
                "Serialized {}: {} objects, {} types, Unity {}",
                serialized.report().format_version,
                serialized.report().objects.len(),
                serialized.report().types.len(),
                serialized.report().unity_version
            );
            println!("Class counts: {classes:?}");
        }
        return Ok(());
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(bundle.report())
                .map_err(|error| format!("could not serialize bundle report: {error}"))?
        );
    } else {
        println!(
            "UnityRaw {}: {} entries, engine {}",
            bundle.report().format_version,
            bundle.report().entries.len(),
            bundle.report().engine_version
        );
    }
    Ok(())
}

fn run_scenario(args: Vec<OsString>) -> Result<(), String> {
    let mut case = None;
    let mut bundle = None;
    let mut json = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].to_str() {
            Some("--case") | Some("--bundle") => {
                let is_case = args[index] == "--case";
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "scenario option requires a path".to_owned())?;
                let destination = if is_case { &mut case } else { &mut bundle };
                if destination.replace(PathBuf::from(value)).is_some() {
                    return Err("scenario paths may only be supplied once".to_owned());
                }
            }
            Some("--json") => json = true,
            _ => {
                return Err(format!(
                    "unrecognized scenario argument: {}",
                    args[index].to_string_lossy()
                ));
            }
        }
        index += 1;
    }
    let case = case.ok_or_else(|| "missing required `--case <path>`".to_owned())?;
    let mut spec =
        tantalus_tools::scenario::ScenarioRunner::read(&case).map_err(|error| error.to_string())?;
    if let Some(bundle) = bundle {
        spec.bundle = bundle;
    }
    let report =
        tantalus_tools::scenario::ScenarioRunner::run(&spec).map_err(|error| error.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| format!("could not serialize scenario report: {error}"))?
        );
    } else {
        println!(
            "{}: {} ticks, final position {:?}, triangle {}",
            report.field, report.ticks, report.final_position, report.final_triangle
        );
    }
    if report.passed {
        Ok(())
    } else {
        Err(format!(
            "scenario expectation failed: expected {:?} on triangle {}, got {:?} on triangle {}",
            spec.expected.position,
            spec.expected.triangle_index,
            report.final_position,
            report.final_triangle
        ))
    }
}

/// `--path` is a directory holding `p0data11.bin`-`p0data19.bin` or one such
/// bundle. Writes the per-field JSON and, with `png`, camera 0 of every 40th
/// field plus any failing field.
fn run_field_composition(
    path: &std::path::Path,
    out: Option<PathBuf>,
    png: Option<PathBuf>,
) -> Result<(), String> {
    use tantalus_assets::pc::field_composition::compose_bundle;
    use tantalus_assets::pc::ui_atlas::encode_png_rgba;
    let bundles: Vec<PathBuf> = if path.is_dir() {
        (11..=19)
            .map(|n| path.join(format!("p0data{n}.bin")))
            .collect()
    } else {
        vec![path.to_path_buf()]
    };
    if let Some(dir) = &png {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    }
    let mut fields = Vec::new();
    let mut bundle_errors = Vec::new();
    let mut locale_variants = Vec::new();
    for bundle in &bundles {
        let name = bundle
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let result = compose_bundle(bundle, png.is_some(), |composed| {
            let take = fields.len() % 40 == 0 || !composed.report.passed;
            if let (Some(dir), true) = (&png, take) {
                for (camera, raster) in &composed.rasters {
                    let file = dir.join(format!("{}_cam{camera}.png", composed.report.field));
                    let _ = std::fs::write(
                        file,
                        encode_png_rgba(raster.width, raster.height, &raster.rgba),
                    );
                }
            }
            let mut report = composed.report;
            report.bundle = name.clone();
            fields.push(report);
        });
        match result {
            Ok(outcome) => locale_variants.extend(outcome.locale_variants),
            Err(error) => bundle_errors
                .push(serde_json::json!({ "bundle": name, "error": error.to_string() })),
        }
    }
    fields.sort_by(|a, b| a.field.cmp(&b.field));
    let passed = fields.iter().filter(|f| f.passed).count();
    let failed = fields.len() - passed;
    let total = serde_json::json!({
        "fields": fields.len(),
        "passed": passed,
        "failed": failed,
        "bundle_errors": bundle_errors.len(),
        "locale_variants": locale_variants.len(),
        "locale_variants_with_problems": locale_variants
            .iter()
            .filter(|v| !v.problems.is_empty() || v.unaccounted_bytes > 0)
            .count(),
        "pass": failed == 0 && bundle_errors.is_empty() && !fields.is_empty(),
        "padding_bytes": fields.iter().map(|f| f.padding_bytes).sum::<usize>(),
        "overlays": fields.iter().map(|f| f.overlay_count).sum::<usize>(),
        "layers": fields.iter().map(|f| f.layer_count).sum::<usize>(),
        "tiles": fields.iter().map(|f| f.tile_count).sum::<usize>(),
        "non_empty_pixels": fields.iter().map(|f| f.non_empty_pixels).sum::<usize>(),
        "out_of_bounds_tiles": fields.iter().map(|f| f.out_of_bounds_tiles).sum::<usize>(),
        "tiles_outside_overlay": fields.iter().map(|f| f.tiles_outside_overlay).sum::<usize>(),
        "animations": fields.iter().map(|f| f.animation_count).sum::<usize>(),
        "animation_frames": fields.iter().map(|f| f.animation_frame_count).sum::<usize>(),
        "unconsumed_animations": fields.iter().map(|f| f.unconsumed_animations.len()).sum::<usize>(),
        "unaccounted_bytes": fields.iter().map(|f| f.unaccounted_bytes).sum::<usize>(),
        "overlapping_bytes": fields.iter().map(|f| f.overlapping_bytes).sum::<usize>(),
    });
    let report = serde_json::json!({
        "total": total,
        "bundle_errors": bundle_errors,
        "locale_variants": locale_variants,
        "fields": fields,
    });
    let target = out.unwrap_or_else(|| PathBuf::from("audit/field-composition.json"));
    std::fs::write(
        &target,
        serde_json::to_string_pretty(&report).unwrap()
            + "
",
    )
    .map_err(|error| format!("could not write {}: {error}", target.display()))?;
    println!(
        "field composition: {} fields, {passed} passed, {failed} failed, {} bundle errors -> {}",
        fields.len(),
        bundle_errors.len(),
        target.display()
    );
    println!(
        "{}",
        if total["pass"] == true {
            "PASS"
        } else {
            "FAIL"
        }
    );
    Ok(())
}

/// `audit summary [--audit-dir <dir>] [--json] [--deny-fail] | audit compare-sweeps <before.json> <after.json> [--json]`: pass and fail counts per asset class
/// from the committed audit reports (milestone W9).
/// `audit compare-sweeps <before.json> <after.json> [--json]`: which battles
/// changed outcome, spoils, or errors between two sweep reports.
fn run_compare_sweeps(args: Vec<OsString>) -> Result<(), String> {
    let mut json = false;
    let mut paths = Vec::new();
    let mut index = 2;
    while index < args.len() {
        match args[index].to_str() {
            Some("--json") => json = true,
            Some(other) if other.starts_with("--") => return Err(usage().to_owned()),
            Some(other) => paths.push(other.to_owned()),
            None => return Err("paths must be UTF-8".to_owned()),
        }
        index += 1;
    }
    if paths.len() != 2 {
        return Err("compare-sweeps requires exactly two report paths".to_owned());
    }
    let read = |path: &str| -> Result<serde_json::Value, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {path}: {error}"))?;
        serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))
    };
    let report = tantalus_tools::sweep_compare::compare(&read(&paths[0])?, &read(&paths[1])?);
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else if report["identical"].as_bool() == Some(true) {
        println!(
            "{} battles: identical",
            report["battles"].as_u64().unwrap_or(0)
        );
    } else {
        println!(
            "{} battles: {} outcome, {} spoils, {} error changes",
            report["battles"].as_u64().unwrap_or(0),
            report["outcome_changes"].as_u64().unwrap_or(0),
            report["spoils_changes"].as_u64().unwrap_or(0),
            report["error_changes"].as_u64().unwrap_or(0),
        );
        for delta in report["deltas"].as_array().cloned().unwrap_or_default() {
            println!(
                "  {} p{} {:?}: {:?} -> {:?}",
                delta["scene"],
                delta["pattern"],
                delta["kinds"],
                delta["before"]["outcome"],
                delta["after"]["outcome"]
            );
        }
    }
    Ok(())
}

fn run_audit_summary(args: Vec<OsString>) -> Result<(), String> {
    let mut dir = PathBuf::from("audit");
    let mut json = false;
    let mut deny_fail = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].to_str() {
            Some("--audit-dir") => {
                index += 1;
                dir = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "--audit-dir requires a path".to_owned())?,
                );
            }
            Some("--json") => json = true,
            Some("--deny-fail") => deny_fail = true,
            _ => return Err(usage().to_owned()),
        }
        index += 1;
    }
    let rows = tantalus_tools::audit_summary::summarize(&dir);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&tantalus_tools::audit_summary::to_json(&rows))
                .map_err(|error| error.to_string())?
        );
    } else {
        print!("{}", tantalus_tools::audit_summary::render_table(&rows));
    }
    let (failing, missing) = tantalus_tools::audit_summary::failure_counts(&rows);
    if missing > 0 {
        return Err(format!(
            "{missing} audit report row(s) missing or unreadable"
        ));
    }
    if deny_fail && failing > 0 {
        return Err(format!("{failing} asset class(es) have failures"));
    }
    Ok(())
}

fn usage() -> &'static str {
    "expected `install inspect --root <path> [--json]`, `audit summary [--audit-dir <dir>] [--json] [--deny-fail] | audit compare-sweeps <before.json> <after.json> [--json]`, `assets inspect-serialized --path <file> [--json] [--out <path>] [--mes <name>|--mes-all] [--ui-atlases] [--atlas-png <name> <out>] [--fonts [--font-png <dir>]] [--render-dialog <mes name> <entry> <out.png> [--dialog-variant <n>] [--dialog-page <n>] [--dialog-window blue|gray] [--dialog-size auto|hint]]`, `assets inspect-movie --dir <dir> [--verify-crc] [--decode-frames <name> <n|all> [--png <dir>]] [--out <path>] [--json]`, `assets battle-tables --dll <Assembly-CSharp.dll> [--out <audit json>] [--tables-out <full json>] [--json]`, `assets import --plan <path> [--json]`, `assets inspect-bundle --path <path> [--objects] [--text-asset <name>] [--inventory] [--mbg-all] [--audio-all] [--map-config-all] [--audio-decode [--audio-out <path>] [--wav <dir>] [--resources-assets <path>] [--movie-inventory <path>]] [--field-models [--models <p0data4.bin>]] [--field-composition [--composition-out <json>] [--png <dir>]] [--field-animation-check [--animation-out <json>]] [--field-animation-frames <field> <tick> <out.png> [--animation-camera <n>]] [--assemble-model <dir>] [--assemble-all] [--clips <p0data5.bin>] [--clip-sampling <out json>] [--clip-durations <out json> --clips <p0data5.bin>] [--world-map] [--world-map-audit <path>] [--world-walk <out json>] [--battle-sequences <out json>] [--battle-model-coverage <out json> [--models <p0data4.bin>] [--clips <p0data5.bin>] [--dll <Assembly-CSharp.dll>] [--resources-assets <path>]] [--special-effects <out json> [--dll <Assembly-CSharp.dll>] [--resources-assets <path>]] [--unity-objects [--class <id>]] [--event <name> [--probe]] [--texture <name>] [--texture-id <path-id>] [--rgba-output <path>] [--bgi <name>] [--bgs <name>] [--battle-scenes] [--battle-scripts-probe <out json>] [--battle-scene <EVT_BATTLE_name> [--battle-ai-demo <scene|first-random> <out json>] [--battle-rewards] [--battle-run --scripts-bundle <p0data7.bin>] [--battle-run-all <out json> --scripts-bundle <p0data7.bin>] [--battle-world-run-all <out json> --scripts-bundle <p0data7.bin>] [--battle-seed <u64>] [--battle-weak-party] [--battle-flee-first] [--battle-run-everything <out dir>]] [--field <name>] [--atlas-for <field>] [--json]`, or `scenario run --case <path> [--bundle <path>] [--json]`"
}

fn print_help() {
    println!(
        "tantalusctl\n\nUSAGE:\n    tantalusctl install inspect --root <path> [--json]\n    tantalusctl audit summary [--audit-dir <dir>] [--json] [--deny-fail] | audit compare-sweeps <before.json> <after.json> [--json]\n    tantalusctl assets inspect-serialized --path <file> [--json] [--out <path>] [--mes <name>|--mes-all] [--ui-atlases] [--atlas-png <name> <out>] [--fonts [--font-png <dir>]] [--render-dialog <mes name> <entry> <out.png> [--dialog-variant <n>] [--dialog-page <n>] [--dialog-window blue|gray] [--dialog-size auto|hint]]\n    tantalusctl assets inspect-movie --dir <dir> [--verify-crc] [--decode-frames <name> <n|all> [--png <dir>]] [--out <path>] [--json]\n    tantalusctl assets battle-tables --dll <Assembly-CSharp.dll> [--out <audit json>] [--tables-out <full json>] [--json]\n    tantalusctl assets import --plan <path> [--json]\n    tantalusctl assets inspect-bundle --path <path> [--objects] [--text-asset <name>] [--inventory] [--mbg-all] [--map-config-all] [--audio-decode [--audio-out <path>] [--wav <dir>] [--resources-assets <path>] [--movie-inventory <path>]] [--field-models [--models <p0data4.bin>]] [--field-composition [--composition-out <json>] [--png <dir>]] [--field-animation-check [--animation-out <json>]] [--field-animation-frames <field> <tick> <out.png> [--animation-camera <n>]] [--assemble-model <dir>] [--assemble-all] [--clips <p0data5.bin>] [--clip-sampling <out json>] [--clip-durations <out json> --clips <p0data5.bin>] [--world-map] [--world-map-audit <path>] [--world-walk <out json>] [--battle-sequences <out json>] [--battle-model-coverage <out json> [--models <p0data4.bin>] [--clips <p0data5.bin>] [--dll <Assembly-CSharp.dll>] [--resources-assets <path>]] [--special-effects <out json> [--dll <Assembly-CSharp.dll>] [--resources-assets <path>]] [--audio-all] [--unity-objects [--class <id>]] [--event <name> [--probe]] [--texture <name>] [--texture-id <path-id>] [--rgba-output <path>] [--bgi <name>] [--bgs <name>] [--battle-scenes] [--battle-scripts-probe <out json>] [--battle-scene <EVT_BATTLE_name> [--battle-ai-demo <scene|first-random> <out json>] [--battle-rewards] [--battle-run --scripts-bundle <p0data7.bin>] [--battle-run-all <out json> --scripts-bundle <p0data7.bin>] [--battle-world-run-all <out json> --scripts-bundle <p0data7.bin>] [--battle-seed <u64>] [--battle-weak-party] [--battle-flee-first] [--battle-run-everything <out dir>]] [--field <name>] [--atlas-for <field>] [--json]\n    tantalusctl scenario run --case <path> [--bundle <path>] [--json]\n\nInspect installations and UnityRaw bundles, import validated local asset packs, or run typed headless scenarios."
    );
}

/// `--battle-scenes` (whole bundle) or `--battle-scene <name>` (one scene), always JSON.
fn run_battle_scripts_probe(
    bundle: &tantalus_assets::pc::unity_raw::UnityRawBundle,
    path: &std::path::Path,
    out: &std::path::Path,
    json: bool,
) -> Result<(), String> {
    use std::collections::BTreeMap;
    type Histogram = BTreeMap<String, (usize, String)>;
    fn bump(map: &mut Histogram, key: String, example: &str) {
        map.entry(key).or_insert_with(|| (0, example.to_owned())).0 += 1;
    }
    fn render(map: &Histogram) -> Vec<serde_json::Value> {
        map.iter()
            .map(|(key, (count, example))| {
                serde_json::json!({"key": key, "count": count, "first_at": example})
            })
            .collect()
    }
    let bytes = bundle
        .read_entry(0, 300 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    let serialized =
        tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
            .map_err(|error| error.to_string())?;
    let mut scripts = 0_usize;
    let mut parse_failures = Vec::new();
    let (mut decoded, mut decode_errors, mut empty) = (0_usize, 0_usize, 0_usize);
    let (mut probed, mut completed, mut waiting, mut budget, mut refusals) =
        (0_usize, 0_usize, 0_usize, 0_usize, 0_usize);
    let mut unsupported: Histogram = BTreeMap::new();
    let mut runtime_kinds: Histogram = BTreeMap::new();
    let mut decode_kinds: Histogram = BTreeMap::new();
    let mut events: BTreeMap<String, usize> = BTreeMap::new();
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut waits: Histogram = BTreeMap::new();
    let mut sysvars: Histogram = BTreeMap::new();
    let mut syslist: Histogram = BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    for (index, object) in serialized.report().objects.iter().enumerate() {
        if object.class_id != 49 {
            continue;
        }
        let asset = tantalus_assets::pc::text_asset::TextAsset::parse(
            serialized
                .object_data(index)
                .map_err(|error| error.to_string())?,
            "battle-script",
        )
        .map_err(|error| error.to_string())?;
        if !asset.name.starts_with("EVT_BATTLE_") || !seen.insert(asset.name.clone()) {
            continue;
        }
        scripts += 1;
        let binary = match tantalus_legacy::EventBinary::parse(&asset.script) {
            Ok(binary) => binary,
            Err(error) => {
                parse_failures.push(format!("{}: {error}", asset.name));
                continue;
            }
        };
        for entry in &binary.entries {
            let decoded_entry = match binary.decode_entry_with_diagnostics(entry.index) {
                Ok(value) => value,
                Err(error) => {
                    decode_errors += 1;
                    let at = format!("{} entry {}", asset.name, entry.index);
                    bump(
                        &mut decode_kinds,
                        format!("{:?} opcode {:?}", error.kind, error.opcode),
                        &at,
                    );
                    continue;
                }
            };
            for failure in &decoded_entry.errors {
                decode_errors += 1;
                let at = format!(
                    "{} entry {} fn {}",
                    asset.name, entry.index, failure.function
                );
                bump(
                    &mut decode_kinds,
                    format!("{:?} opcode {:?}", failure.error.kind, failure.error.opcode),
                    &at,
                );
            }
            for function in &decoded_entry.functions {
                decoded += 1;
                if function.program.instructions.is_empty() {
                    empty += 1;
                    continue;
                }
                probed += 1;
                let at = format!("{} entry {} tag {}", asset.name, entry.index, function.tag);
                if let Ok(tree) = serde_json::to_value(&function.program.instructions) {
                    tally_program(&tree, &at, &mut kinds, &mut sysvars, &mut syslist);
                }
                let outcome = tantalus_legacy::probe_program(
                    &function.program,
                    tantalus_contracts::EntityId::new(entry.index as u64),
                    64,
                );
                for (name, count) in outcome.events {
                    *events.entry(name).or_insert(0) += count;
                }
                match outcome.error {
                    Some(failure) => match failure.kind {
                        tantalus_legacy::VmErrorKind::UnsupportedOpcode => {
                            refusals += 1;
                            bump(
                                &mut unsupported,
                                format!("opcode {:?}: {}", failure.opcode, failure.message),
                                &at,
                            );
                        }
                        tantalus_legacy::VmErrorKind::InstructionLimit => budget += 1,
                        kind => bump(
                            &mut runtime_kinds,
                            format!("{kind:?} opcode {:?}: {}", failure.opcode, failure.message),
                            &at,
                        ),
                    },
                    None if outcome.status == tantalus_legacy::VmStatus::Completed => {
                        completed += 1;
                    }
                    None => {
                        waiting += 1;
                        let wait = outcome
                            .wait
                            .map(|wait| format!("{wait:?}"))
                            .and_then(|text| text.split(['(', ' ']).next().map(str::to_owned))
                            .unwrap_or_else(|| "none".to_owned());
                        bump(&mut waits, wait, &at);
                    }
                }
            }
        }
    }
    let report = serde_json::json!({
        "scope": "zero-state dry run of every function of every EVT_BATTLE_* script; permissive host, reads return zero, waits finish; not a gameplay claim",
        "max_ticks": 64,
        "scripts": scripts,
        "script_parse_failures": parse_failures,
        "functions_decoded": decoded,
        "decode_errors": decode_errors,
        "decode_error_kinds": render(&decode_kinds),
        "empty_functions": empty,
        "functions_probed": probed,
        "completed": completed,
        "still_waiting": waiting,
        "instruction_budget_stops": budget,
        "unsupported_opcode_refusals": refusals,
        "unsupported_opcodes": render(&unsupported),
        "other_runtime_error_kinds": render(&runtime_kinds),
        "events": events,
        "wait_states": render(&waits),
        "instruction_kinds": kinds,
        "system_variables_read": render(&sysvars),
        "system_list_elements": render(&syslist),
    });
    let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(out, format!("{text}\n")).map_err(|error| error.to_string())?;
    if json {
        println!("{text}");
    } else {
        println!(
            "{scripts} battle scripts probed; report written to {}",
            out.display()
        );
    }
    Ok(())
}

/// Counts instruction kinds and the system-variable and system-list ids that decoded
/// programs reference, by walking the serialized instruction tree.
fn tally_program(
    tree: &serde_json::Value,
    at: &str,
    kinds: &mut std::collections::BTreeMap<String, usize>,
    sysvars: &mut std::collections::BTreeMap<String, (usize, String)>,
    syslist: &mut std::collections::BTreeMap<String, (usize, String)>,
) {
    fn walk(
        value: &serde_json::Value,
        at: &str,
        sysvars: &mut std::collections::BTreeMap<String, (usize, String)>,
        syslist: &mut std::collections::BTreeMap<String, (usize, String)>,
    ) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, inner) in map {
                    let target = match key.as_str() {
                        "system_variable" => Some(&mut *sysvars),
                        "system_list" => Some(&mut *syslist),
                        _ => None,
                    };
                    if let (Some(target), Some(id)) = (target, inner.as_u64()) {
                        target
                            .entry(id.to_string())
                            .or_insert_with(|| (0, at.to_owned()))
                            .0 += 1;
                    }
                    walk(inner, at, sysvars, syslist);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, at, sysvars, syslist);
                }
            }
            _ => {}
        }
    }
    if let serde_json::Value::Object(instructions) = tree {
        for instruction in instructions.values() {
            let kind = match instruction.get("kind") {
                Some(serde_json::Value::String(name)) => Some(name.clone()),
                Some(serde_json::Value::Object(map)) => map.keys().next().cloned(),
                _ => None,
            };
            if let Some(kind) = kind {
                *kinds.entry(kind).or_insert(0) += 1;
            }
        }
    }
    walk(tree, at, sysvars, syslist);
}

/// `--battle-sequences <out>`: decode and interpret every `.raw17` in the bundle and write
/// the audit document.
fn run_battle_sequences(path: &std::path::Path, out: &std::path::Path) -> Result<(), String> {
    use tantalus_assets::pc::battle_scene::BattleBundleData;
    let data = BattleBundleData::load(path).map_err(|error| error.to_string())?;
    let report = tantalus_assets::pc::battle_sequence::audit(&data.sequence_bytes);
    let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(out, format!("{text}\n")).map_err(|error| error.to_string())?;
    println!(
        "{} sequence files decoded; report written to {}",
        data.sequence_bytes.len(),
        out.display()
    );
    Ok(())
}

fn run_battle_scene_report(
    path: &std::path::Path,
    one: Option<&str>,
    rewards: bool,
) -> Result<(), String> {
    use tantalus_assets::pc::battle_scene::BattleBundleData;
    let data = BattleBundleData::load(path).map_err(|error| error.to_string())?;
    if let Some(name) = one {
        let (key, scene) = data
            .scene(name)
            .ok_or_else(|| format!("battle scene `{name}` was not found"))?;
        let scene = scene.as_ref().map_err(|error| error.to_string())?;
        let sequence = data.sequences.get(key).map(|(file, parsed)| match parsed {
            Ok(header) => serde_json::json!({"file": file, "header": header}),
            Err(error) => serde_json::json!({"file": file, "error": error.to_string()}),
        });
        let reward_rows = if rewards {
            Some(
                scene
                    .patterns
                    .iter()
                    .enumerate()
                    .map(|(index, pattern)| {
                        use tantalus_tools::battle_scene_rewards::{
                            pattern_machine_units, pattern_rewards, reward_totals,
                        };
                        let rows = pattern_rewards(scene, index)
                            .unwrap_or_default()
                            .iter()
                            .map(|row| {
                                serde_json::json!({
                                    "type_id": row.type_id,
                                    "present": row.present,
                                    "gil": row.gil,
                                    "exp": row.exp,
                                    "items": row.items,
                                    "item_rates": row.item_rates,
                                    "card": row.card,
                                    "card_rate": row.card_rate,
                                })
                            })
                            .collect::<Vec<_>>();
                        let (gil, exp) =
                            reward_totals(&pattern_rewards(scene, index).unwrap_or_default());
                        let units = pattern_machine_units(scene, index).unwrap_or_default();
                        let zero_will = units.iter().filter(|unit| unit.will == 0).count();
                        let zero_hp = units.iter().filter(|unit| unit.max_hp == 0).count();
                        serde_json::json!({
                            "pattern": index,
                            "ap": pattern.ap,
                            "enemies": rows,
                            "totals": {"gil": gil, "exp": exp},
                            "machine_units": {
                                "count": units.len(),
                                "zero_will": zero_will,
                                "zero_hp": zero_hp,
                            },
                        })
                    })
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        let report = serde_json::json!({
            "name": key,
            "flags": scene.header.flag_names(),
            "scene": scene,
            "sequence": sequence,
            "rewards": reward_rows,
        });
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return Ok(());
    }
    let mut parsed = 0_usize;
    let mut failed = Vec::new();
    let mut trailing = Vec::new();
    let mut versions: std::collections::BTreeMap<u8, usize> = std::collections::BTreeMap::new();
    let (mut patterns, mut types, mut attacks) = (0_usize, 0_usize, 0_usize);
    for (name, scene) in &data.scenes {
        match scene {
            Ok(scene) => {
                parsed += 1;
                patterns += scene.patterns.len();
                types += scene.monster_types.len();
                attacks += scene.attacks.len();
                *versions.entry(scene.header.version).or_insert(0) += 1;
                if scene.trailing_bytes != 0 {
                    trailing.push(
                        serde_json::json!({"scene": name, "trailing_bytes": scene.trailing_bytes, "all_zero": scene.trailing_all_zero, "nonzero_bytes_after_first_dword": scene.trailing.iter().skip(4).filter(|b| **b != 0).count(), "head": scene.trailing.iter().take(16).map(|b| format!("{b:02x}")).collect::<String>(), "file_len": scene.file_len}),
                    );
                }
            }
            Err(error) => {
                failed.push(serde_json::json!({"scene": name, "error": error.to_string()}))
            }
        }
    }
    let seq_ok = data.sequences.values().filter(|(_, r)| r.is_ok()).count();
    let seq_failed: Vec<_> = data
        .sequences
        .iter()
        .filter_map(|(name, (_, r))| {
            r.as_ref()
                .err()
                .map(|e| serde_json::json!({"scene": name, "error": e.to_string()}))
        })
        .collect();
    let info_ok = data.infos.values().filter(|r| r.is_ok()).count();
    let info_trailing = data
        .infos
        .values()
        .filter(|r| r.as_ref().is_ok_and(|info| info.trailing_bytes != 0))
        .count();
    let info_failed: Vec<_> = data
        .infos
        .iter()
        .filter_map(|(name, r)| {
            r.as_ref()
                .err()
                .map(|e| serde_json::json!({"info": name, "error": e.to_string()}))
        })
        .collect();
    let report = serde_json::json!({
        "scenes_total": data.scenes.len(),
        "scenes_parsed": parsed,
        "scenes_failed": failed.len(),
        "failures": failed,
        "trailing_bytes_cases": trailing,
        "header_versions": versions,
        "total_patterns": patterns,
        "total_monster_types": types,
        "total_attacks": attacks,
        "sequences_parsed": seq_ok,
        "sequences_failed": seq_failed,
        "battle_infos_parsed": info_ok,
        "battle_infos_with_trailing_bytes": info_trailing,
        "battle_infos_failed": info_failed,
        "unknown_fields": tantalus_assets::pc::battle_scene::UNKNOWN_FIELDS,
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    Ok(())
}

/// Runs one battle scene to completion with battle-script enemy AI and
/// prints the JSON report. Requires `--battle-scene <name>` and
/// `--scripts-bundle <p0data7.bin>` (battle event scripts live there).
/// Runs every battle scene in the bundle to completion and writes a JSON
/// report. Requires `--scripts-bundle <p0data7.bin>`.
fn run_battle_run_all(
    path: &std::path::Path,
    scripts: Option<&std::path::Path>,
    out: &std::path::Path,
    all_patterns: bool,
    seed: u64,
    weak_party: bool,
    flee_first: bool,
) -> Result<(), String> {
    run_battle_sweep(
        path,
        scripts,
        out,
        all_patterns,
        seed,
        weak_party,
        flee_first,
        "--battle-run-all",
        tantalus_tools::battle_run::run_scene_battle_caught,
    )
}

/// Regenerates the whole conformance matrix in one process: armed / weak /
/// flee-first × report path / world hand-off, shared bundle parse, then a
/// delta report against the committed audits when they exist. This is the
/// standard post-landing triage command.
fn run_battle_run_everything(
    path: &std::path::Path,
    scripts: Option<&std::path::Path>,
    out_dir: &std::path::Path,
    seed: u64,
) -> Result<(), String> {
    use tantalus_assets::pc::battle_scene::BattleBundleData;
    use tantalus_tools::battle_ai_demo::load_battle_scripts;
    let scripts_path = scripts.ok_or_else(|| {
        "`--battle-run-everything` requires `--scripts-bundle <p0data7.bin>`".to_owned()
    })?;
    let data = BattleBundleData::load(path).map_err(|error| error.to_string())?;
    let table = load_battle_scripts(scripts_path)?;
    let combos: [(&str, bool, bool, SweepRunner); 6] = [
        (
            "battle-run-all-patterns.json",
            false,
            false,
            tantalus_tools::battle_run::run_scene_battle_caught,
        ),
        (
            "battle-world-run-all-patterns.json",
            false,
            false,
            tantalus_tools::battle_run::run_world_battle_caught,
        ),
        (
            "battle-run-all-patterns-weak.json",
            true,
            false,
            tantalus_tools::battle_run::run_scene_battle_caught,
        ),
        (
            "battle-world-run-all-patterns-weak.json",
            true,
            false,
            tantalus_tools::battle_run::run_world_battle_caught,
        ),
        (
            "battle-run-all-patterns-flee.json",
            false,
            true,
            tantalus_tools::battle_run::run_scene_battle_caught,
        ),
        (
            "battle-world-run-all-patterns-flee.json",
            false,
            true,
            tantalus_tools::battle_run::run_world_battle_caught,
        ),
    ];
    std::fs::create_dir_all(out_dir).map_err(|error| error.to_string())?;
    for (file, weak, flee, runner) in combos {
        let results = sweep_corpus(&data, &table, true, seed, weak, flee, runner)?;
        let mut flat = Vec::new();
        for result in &results {
            if let Some(reports) = result.get("patterns").and_then(|p| p.as_array()) {
                flat.extend(reports.iter().cloned());
            } else {
                flat.push(result.clone());
            }
        }
        let count = |outcome: &str| flat.iter().filter(|r| r["outcome"] == outcome).count();
        let summary = serde_json::json!({
            "scenes": results.len(),
            "battles": flat.len(),
            "victories": count("victory"),
            "defeats": count("defeat"),
            "escapes": count("escape"),
            "errors": flat.iter().filter(|r| r.get("error").is_some_and(|e| !e.is_null())).count(),
            "results": results,
        });
        let text = serde_json::to_string_pretty(&summary).map_err(|error| error.to_string())?;
        let out_path = out_dir.join(file);
        std::fs::write(
            &out_path,
            format!(
                "{text}
"
            ),
        )
        .map_err(|error| error.to_string())?;
        println!(
            "{file}: {} battles, {} V / {} D / {} E / {} errors",
            summary["battles"],
            summary["victories"],
            summary["defeats"],
            summary["escapes"],
            summary["errors"]
        );
        let committed = std::path::Path::new("audit").join(file);
        if committed.exists()
            && let Ok(committed_text) = std::fs::read_to_string(&committed)
            && let (Ok(committed_json), Ok(fresh_json)) = (
                serde_json::from_str::<serde_json::Value>(&committed_text),
                serde_json::from_str::<serde_json::Value>(&text),
            )
        {
            let report = tantalus_tools::sweep_compare::compare(&committed_json, &fresh_json);
            println!(
                "  vs committed: identical={} ({} outcome / {} spoils / {} error deltas)",
                report["identical"],
                report["outcome_changes"],
                report["spoils_changes"],
                report["error_changes"]
            );
        }
    }
    Ok(())
}

/// World-hand-off twin of [`run_battle_run_all`]: every scene/pattern runs
/// through `run_world_battle` (party from a scratch field world, exactly-once
/// reward commit, HP/MP carry-back) so both engine paths can be cross-checked
/// over the whole encounter corpus with `audit compare-sweeps`.
fn run_battle_world_run_all(
    path: &std::path::Path,
    scripts: Option<&std::path::Path>,
    out: &std::path::Path,
    all_patterns: bool,
    seed: u64,
    weak_party: bool,
    flee_first: bool,
) -> Result<(), String> {
    run_battle_sweep(
        path,
        scripts,
        out,
        all_patterns,
        seed,
        weak_party,
        flee_first,
        "--battle-world-run-all",
        tantalus_tools::battle_run::run_world_battle_caught,
    )
}

#[allow(clippy::too_many_arguments)] // CLI plumbing: bundle struct would churn both callers for no gain.
type SweepRunner = fn(
    scene_name: &str,
    scene: &tantalus_assets::pc::battle_scene::BattleScene,
    pattern: usize,
    script: &[u8],
    seed: u64,
    weak_party: bool,
    flee_first: bool,
) -> tantalus_tools::battle_run::BattleRunReport;

/// Runs the full corpus once over preloaded bundle data. Shared by the
/// single-mode sweeps and `--battle-run-everything`.
fn sweep_corpus(
    data: &tantalus_assets::pc::battle_scene::BattleBundleData,
    table: &std::collections::BTreeMap<String, Vec<u8>>,
    all_patterns: bool,
    seed: u64,
    weak_party: bool,
    flee_first: bool,
    run: SweepRunner,
) -> Result<Vec<serde_json::Value>, String> {
    let mut results = Vec::new();
    for (name, scene) in &data.scenes {
        let entry = match scene {
            Err(error) => {
                serde_json::json!({"scene": name, "error": format!("scene parse: {error}")})
            }
            Ok(scene) => match table.get(name) {
                None => serde_json::json!({"scene": name, "error": "no battle script"}),
                Some(bytes) => {
                    use tantalus_tools::battle_scene_rewards::choose_pattern;
                    if all_patterns {
                        let mut reports = Vec::new();
                        for pattern in 0..scene.patterns.len() {
                            let report =
                                run(name, scene, pattern, bytes, seed, weak_party, flee_first);
                            reports.push(serde_json::to_value(report).map_err(|e| e.to_string())?);
                        }
                        serde_json::json!({"scene": name, "patterns": reports})
                    } else {
                        let pattern = choose_pattern(scene, None, 50);
                        let report = run(name, scene, pattern, bytes, seed, weak_party, flee_first);
                        serde_json::to_value(report).map_err(|error| error.to_string())?
                    }
                }
            },
        };
        results.push(entry);
    }
    Ok(results)
}

#[allow(clippy::too_many_arguments)] // CLI plumbing: bundle struct would churn both callers for no gain.
fn run_battle_sweep(
    path: &std::path::Path,
    scripts: Option<&std::path::Path>,
    out: &std::path::Path,
    all_patterns: bool,
    seed: u64,
    weak_party: bool,
    flee_first: bool,
    flag: &str,
    run: SweepRunner,
) -> Result<(), String> {
    use tantalus_assets::pc::battle_scene::BattleBundleData;
    use tantalus_tools::battle_ai_demo::load_battle_scripts;
    let scripts_path =
        scripts.ok_or_else(|| format!("`{flag}` requires `--scripts-bundle <p0data7.bin>`"))?;
    let data = BattleBundleData::load(path).map_err(|error| error.to_string())?;
    let table = load_battle_scripts(scripts_path)?;
    let results = sweep_corpus(
        &data,
        &table,
        all_patterns,
        seed,
        weak_party,
        flee_first,
        run,
    )?;
    // Flatten per-pattern entries for counting.
    let mut flat = Vec::new();
    for result in &results {
        if let Some(reports) = result.get("patterns").and_then(|p| p.as_array()) {
            flat.extend(reports.iter().cloned());
        } else {
            flat.push(result.clone());
        }
    }
    let count = |outcome: &str| flat.iter().filter(|r| r["outcome"] == outcome).count();
    let summary = serde_json::json!({
        "scenes": results.len(),
        "battles": flat.len(),
        "victories": count("victory"),
        "defeats": count("defeat"),
        "escapes": count("escape"),
        "errors": flat
            .iter()
            .filter(|r| r.get("error").is_some_and(|e| !e.is_null()))
            .count(),
        "results": results,
    });
    let text = serde_json::to_string_pretty(&summary).map_err(|error| error.to_string())?;
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(
        out,
        format!(
            "{text}
"
        ),
    )
    .map_err(|error| error.to_string())?;
    println!(
        "{} scenes ({} battles): {} victories, {} defeats, {} escapes, {} errors -> {}",
        results.len(),
        flat.len(),
        summary["victories"],
        summary["defeats"],
        summary["escapes"],
        summary["errors"],
        out.display()
    );
    Ok(())
}

/// Runs one real scene battle through a scratch field world (armed level-20
/// party) and prints the outcome with the world state deltas. Requires
/// `--battle-scene <name>` and `--scripts-bundle <p0data7.bin>`.
fn run_battle_world_run(
    path: &std::path::Path,
    one: Option<&str>,
    scripts: Option<&std::path::Path>,
    json: bool,
) -> Result<(), String> {
    use tantalus_assets::pc::battle_scene::BattleBundleData;
    use tantalus_contracts::EntityId;
    use tantalus_game::FieldWorld;
    use tantalus_legacy::battle::WeaponStats;
    use tantalus_tools::battle_ai_demo::load_battle_scripts;
    use tantalus_tools::battle_run::run_world_battle;
    let name = one.ok_or("`--battle-world-run` requires `--battle-scene <name>`")?;
    let scripts_path =
        scripts.ok_or("`--battle-world-run` requires `--scripts-bundle <p0data7.bin>`")?;
    let data = BattleBundleData::load(path).map_err(|error| error.to_string())?;
    let (key, scene) = data
        .scene(name)
        .ok_or_else(|| format!("battle scene `{name}` was not found"))?;
    let scene = scene.as_ref().map_err(|error| error.to_string())?;
    let table = load_battle_scripts(scripts_path)?;
    let bytes = table
        .get(key)
        .ok_or_else(|| format!("battle script `{key}` was not found"))?;

    let mut world = FieldWorld::new();
    world.party = vec![0, 1, 2, 3];
    world.gil = 500;
    // Resolve drawn drops into the inventory: the vanilla table's 256 ids.
    world.item_id_map = Some(tantalus_game::field::world::default_item_id_map(0..=255));
    let entities: Vec<EntityId> = (0..4).map(|index| EntityId::new(101 + index)).collect();
    for character in world.party.clone() {
        world.character_stats.insert(
            character,
            tantalus_game::field::CharacterStats {
                current_hp: 500,
                max_hp: 500,
                current_mp: 100,
                max_mp: 100,
            },
        );
    }
    let base = |_| tantalus_game::battle::legacy::PlayerBase {
        level: 20,
        strength: 30,
        magic: 30,
        dexterity: 30,
        will: 20,
        physical_defence: 10,
        physical_evade: 5,
        magic_defence: 12,
        magic_evade: 6,
        weapon: Some(WeaponStats {
            power: 25,
            rate: 0,
            elements: tantalus_legacy::battle::Elements::NONE,
            category: WeaponStats::SHORT_RANGE,
            status: 0,
        }),
        learnable: Vec::new(),
        equipped_abilities: Vec::new(),
    };
    let outcome = run_world_battle(&mut world, &entities, scene, 0, bytes, &base, 0x5EED, false)
        .map_err(|error| format!("world battle failed: {error}"))?;
    let report = serde_json::json!({
        "scene": key,
        "outcome": outcome,
        "gil": world.gil,
        "gil_delta": world.gil as i64 - 500,
        "character_exp": world.character_exp,
        "items": world.items,
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!(
            "{}: {:?} -> gil {} (delta {}), exp {:?}, items {:?}",
            key,
            outcome,
            report["gil"],
            report["gil_delta"],
            report["character_exp"],
            report["items"]
        );
    }
    Ok(())
}

fn value_next(
    args: &[std::ffi::OsString],
    index: &mut usize,
    message: &str,
) -> Result<std::ffi::OsString, String> {
    *index += 1;
    args.get(*index).cloned().ok_or_else(|| message.to_owned())
}

fn run_battle_run(
    path: &std::path::Path,
    one: Option<&str>,
    scripts: Option<&std::path::Path>,
) -> Result<(), String> {
    use tantalus_assets::pc::battle_scene::BattleBundleData;
    use tantalus_tools::battle_ai_demo::load_battle_scripts;
    use tantalus_tools::battle_run::run_scene_battle;
    let name = one.ok_or("`--battle-run` requires `--battle-scene <name>`")?;
    let scripts_path = scripts.ok_or("`--battle-run` requires `--scripts-bundle <p0data7.bin>`")?;
    let data = BattleBundleData::load(path).map_err(|error| error.to_string())?;
    let (key, scene) = data
        .scene(name)
        .ok_or_else(|| format!("battle scene `{name}` was not found"))?;
    let scene = scene.as_ref().map_err(|error| error.to_string())?;
    let table = load_battle_scripts(scripts_path)?;
    let bytes = table.get(key).ok_or_else(|| {
        format!(
            "battle script `{key}` was not found in {}",
            scripts_path.display()
        )
    })?;
    let report = run_scene_battle(key, scene, 0, bytes, 0x5EED, false, false);
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    Ok(())
}

/// Parses every `mbgdata` and/or `mapconfigdata` container entry in a bundle
/// and prints a JSON summary (parsed/failed counts, frames, unconsumed bytes).
fn run_parse_all(
    bundle: &tantalus_assets::pc::unity_raw::UnityRawBundle,
    path: &std::path::Path,
    mbg_all: bool,
    map_config_all: bool,
) -> Result<(), String> {
    let bytes = bundle
        .read_entry(0, 600 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    let source = path.to_string_lossy();
    let serialized = tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &source)
        .map_err(|error| error.to_string())?;
    let report = serialized.report();
    let by_path_id: std::collections::BTreeMap<i64, usize> = report
        .objects
        .iter()
        .enumerate()
        .map(|(index, object)| (object.path_id, index))
        .collect();
    let (container_index, _) = report
        .objects
        .iter()
        .enumerate()
        .find(|(_, object)| object.class_id == 142)
        .ok_or_else(|| "AssetBundle metadata object is missing".to_owned())?;
    let metadata = tantalus_assets::pc::asset_bundle::AssetBundleMetadata::parse(
        serialized
            .object_data(container_index)
            .map_err(|error| error.to_string())?,
        &source,
    )
    .map_err(|error| error.to_string())?;
    let script_of = |asset_path: &str, path_id: i64| -> Result<Vec<u8>, String> {
        let index = by_path_id
            .get(&path_id)
            .ok_or_else(|| format!("{asset_path}: path ID {path_id} has no object"))?;
        let data = serialized
            .object_data(*index)
            .map_err(|error| error.to_string())?;
        tantalus_assets::pc::text_asset::TextAsset::parse(data, asset_path)
            .map(|asset| asset.script)
            .map_err(|error| error.to_string())
    };
    let mut output = serde_json::Map::new();
    if mbg_all {
        let (mut parsed, mut failed, mut total_frames, mut unconsumed) = (0, 0, 0usize, 0usize);
        let (mut trailing, mut files, mut errors) = (0usize, Vec::new(), Vec::new());
        for (asset_path, info) in &metadata.container {
            if !asset_path.to_ascii_lowercase().ends_with(".mbgdata.bytes") {
                continue;
            }
            let result = script_of(asset_path, info.path_id).and_then(|script| {
                tantalus_assets::pc::mbg::MbgFile::parse(&script, asset_path)
                    .map(|file| (script.len(), file))
                    .map_err(|error| error.to_string())
            });
            match result {
                Ok((size, file)) => {
                    parsed += 1;
                    total_frames += file.frames.len();
                    unconsumed += file.unconsumed_bytes();
                    trailing += file.trailing.len();
                    files.push(serde_json::json!({
                        "path": asset_path,
                        "bytes": size,
                        "frames": file.frames.len(),
                        "declared_frames": file.frame_count,
                        "unread_record_bytes": file.unconsumed_bytes() - file.trailing.len(),
                        "trailing_bytes": file.trailing.len(),
                    }));
                }
                Err(error) => {
                    failed += 1;
                    errors.push(serde_json::json!({"path": asset_path, "error": error}));
                }
            }
        }
        output.insert(
            "mbg".to_owned(),
            serde_json::json!({
                "parsed": parsed,
                "failed": failed,
                "total_frames": total_frames,
                "unconsumed_bytes": unconsumed,
                "trailing_bytes": trailing,
                "failures": errors,
                "files": files,
            }),
        );
    }
    if map_config_all {
        let (mut parsed, mut failed, mut trailing) = (0, 0, 0usize);
        let (mut lights, mut chars, mut errors) = (0usize, 0usize, Vec::new());
        let mut trailing_files = Vec::new();
        let mut versions = std::collections::BTreeMap::<u16, usize>::new();
        for (asset_path, info) in &metadata.container {
            if !asset_path.to_ascii_lowercase().contains("/mapconfigdata/") {
                continue;
            }
            let result = script_of(asset_path, info.path_id).and_then(|script| {
                tantalus_assets::pc::map_config::MapConfig::parse(&script, asset_path)
                    .map(|config| (script.len(), config))
                    .map_err(|error| error.to_string())
            });
            match result {
                Ok((size, config)) => {
                    parsed += 1;
                    lights += config.lights.len();
                    chars += config.chars.len();
                    trailing += config.trailing.len();
                    *versions.entry(config.version).or_default() += 1;
                    if !config.trailing.is_empty() {
                        trailing_files.push(serde_json::json!({
                            "path": asset_path,
                            "bytes": size,
                            "trailing_bytes": config.trailing.len(),
                            "evt_count": config.evt_count,
                            "memoria_size": config.memoria_size(),
                        }));
                    }
                }
                Err(error) => {
                    failed += 1;
                    errors.push(serde_json::json!({"path": asset_path, "error": error}));
                }
            }
        }
        output.insert(
            "map_config".to_owned(),
            serde_json::json!({
                "parsed": parsed,
                "failed": failed,
                "total_lights": lights,
                "total_chars": chars,
                "trailing_bytes": trailing,
                "versions": versions,
                "failures": errors,
                "files_with_trailing_bytes": trailing_files,
            }),
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::Value::Object(output)).unwrap()
    );
    Ok(())
}

/// `assets inspect-serialized --path <p_fa.mpc> --fonts [--font-png <dir>] [--out <json>]`.
fn run_font_inventory(
    path: &Path,
    font_png: Option<PathBuf>,
    out: Option<PathBuf>,
) -> Result<(), String> {
    use tantalus_assets::pc::font_bundle::inventory_encrypted;
    use tantalus_assets::pc::ui_atlas::encode_png_rgba;
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let (mut inventory, pixels) = inventory_encrypted(&bytes).map_err(|error| error.to_string())?;
    if let Some(dir) = &font_png {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
        for (font, blobs) in inventory.fonts.iter().zip(&pixels) {
            if font.sfnt.is_some() {
                let file = format!("{}.ttf", font.name);
                std::fs::write(dir.join(&file), &blobs.font_data)
                    .map_err(|error| format!("could not write {file}: {error}"))?;
            }
        }
        for (font, blobs) in inventory.fonts.iter_mut().zip(&pixels) {
            let rgba = &blobs.rgba;
            let Some(texture) = font.texture.as_mut() else {
                continue;
            };
            if rgba.is_empty() {
                continue;
            }
            let file = format!("{}.png", font.name);
            let png = encode_png_rgba(texture.width as u32, texture.height as u32, rgba);
            std::fs::write(dir.join(&file), png)
                .map_err(|error| format!("could not write {file}: {error}"))?;
            texture.png_file = Some(file);
        }
    }
    let rendered = serde_json::to_string_pretty(&inventory)
        .map_err(|error| format!("could not serialize font inventory: {error}"))?;
    let out = out.unwrap_or_else(|| PathBuf::from("audit/font-inventory.json"));
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(
        &out,
        format!(
            "{rendered}
"
        ),
    )
    .map_err(|error| format!("could not write {}: {error}", out.display()))?;
    println!("{rendered}");
    Ok(())
}

/// `assets inspect-serialized --path <resources.assets> --render-dialog <mes name> <entry> <out.png>
/// [--dialog-variant <n>] [--dialog-page <n>] [--dialog-window blue|gray] [--dialog-size auto|hint]`.
#[allow(clippy::too_many_arguments)]
fn run_render_dialog(
    path: &Path,
    name: &str,
    entry: i32,
    out_png: &Path,
    variant: usize,
    page: usize,
    window: &str,
    size: &str,
) -> Result<(), String> {
    use tantalus_assets::pc::dialog_render::{
        RenderRequest, SizeMode, UNKNOWNS, WindowAtlasChoice, render_from_install,
    };
    use tantalus_assets::pc::ui_atlas::encode_png_rgba;
    let window = match window {
        "blue" => WindowAtlasChoice::Blue,
        "gray" => WindowAtlasChoice::Gray,
        other => {
            return Err(format!(
                "`--dialog-window` must be blue or gray, got `{other}`"
            ));
        }
    };
    let size_mode = match size {
        "auto" => SizeMode::Auto,
        "hint" => SizeMode::Hint,
        other => {
            return Err(format!(
                "`--dialog-size` must be auto or hint, got `{other}`"
            ));
        }
    };
    let rendered = render_from_install(&RenderRequest {
        resources_assets: path,
        mes_name: name,
        variant,
        entry_id: entry,
        page,
        window,
        size_mode,
    })
    .map_err(|error| error.to_string())?;
    let png = encode_png_rgba(
        rendered.image.width as u32,
        rendered.image.height as u32,
        &rendered.image.rgba,
    );
    if let Some(parent) = out_png.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(out_png, png)
        .map_err(|error| format!("could not write {}: {error}", out_png.display()))?;
    let layout = &rendered.layout;
    println!(
        "{name} entry {entry} variant {variant}/{} page {page}/{}: {}x{} px image, body {}x{} ({} sizing), font {}, atlas {}",
        rendered.variant_count,
        layout.pages.len(),
        rendered.image.width,
        rendered.image.height,
        layout.body_width,
        layout.body_height,
        layout.size_mode,
        rendered.font_name,
        rendered.atlas_name
    );
    println!("source: {}", rendered.entry_source.replace('\n', "\\n"));
    println!(
        "recorded, not executed: {} timing markers; {} unhandled tags; {} unknown tags; {} chars without a glyph",
        layout.timing_events.len(),
        layout.unhandled_tags.len(),
        layout.unknown_tags.len(),
        layout.missing_glyphs.len()
    );
    for tag in layout.timing_events.iter().chain(&layout.unhandled_tags) {
        println!(
            "  page {} glyph {} {} {}",
            tag.page, tag.glyph_index, tag.code, tag.raw
        );
    }
    println!("unknowns:");
    for unknown in UNKNOWNS {
        println!("  - {unknown}");
    }
    Ok(())
}

fn run_assets_inspect_serialized(args: Vec<OsString>) -> Result<(), String> {
    use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
    let mut path = None;
    let mut json = false;
    let mut out = None;
    let mut mes = None;
    let mut mes_all = false;
    let mut ui_atlases = false;
    let mut fonts = false;
    let mut font_png: Option<PathBuf> = None;
    let mut atlas_png: Option<(String, PathBuf)> = None;
    let mut render_dialog: Option<(String, i32, PathBuf)> = None;
    let mut dialog_variant = 0usize;
    let mut dialog_page = 0usize;
    let mut dialog_window = "blue".to_owned();
    let mut dialog_size = "auto".to_owned();
    let mut index = 2;
    while index < args.len() {
        match args[index].to_str() {
            Some("--json") => json = true,
            Some("--mes-all") => mes_all = true,
            Some("--ui-atlases") => ui_atlases = true,
            Some("--fonts") => fonts = true,
            Some("--font-png") => {
                index += 1;
                let dir = args
                    .get(index)
                    .ok_or_else(|| "`--font-png` requires <dir>".to_owned())?;
                font_png = Some(PathBuf::from(dir));
            }
            Some("--render-dialog") => {
                let (Some(name), Some(entry), Some(out)) = (
                    args.get(index + 1),
                    args.get(index + 2),
                    args.get(index + 3),
                ) else {
                    return Err(
                        "`--render-dialog` requires <mes name> <entry> <out.png>".to_owned()
                    );
                };
                let entry = entry
                    .to_string_lossy()
                    .parse::<i32>()
                    .map_err(|_| "`--render-dialog` entry must be an integer text id".to_owned())?;
                render_dialog = Some((
                    name.to_string_lossy().into_owned(),
                    entry,
                    PathBuf::from(out),
                ));
                index += 3;
            }
            Some(
                flag @ ("--dialog-variant" | "--dialog-page" | "--dialog-window" | "--dialog-size"),
            ) => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{flag}` requires a value"))?
                    .to_string_lossy()
                    .into_owned();
                match flag {
                    "--dialog-variant" => {
                        dialog_variant = value
                            .parse()
                            .map_err(|_| "`--dialog-variant` must be an integer".to_owned())?;
                    }
                    "--dialog-page" => {
                        dialog_page = value
                            .parse()
                            .map_err(|_| "`--dialog-page` must be an integer".to_owned())?;
                    }
                    "--dialog-window" => dialog_window = value,
                    _ => dialog_size = value,
                }
            }
            Some("--atlas-png") => {
                let (Some(name), Some(out)) = (args.get(index + 1), args.get(index + 2)) else {
                    return Err("`--atlas-png` requires <name> <out>".to_owned());
                };
                atlas_png = Some((name.to_string_lossy().into_owned(), PathBuf::from(out)));
                index += 2;
            }
            Some(flag @ ("--path" | "--out" | "--mes")) => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{flag}` requires a value"))?;
                let slot_taken = match flag {
                    "--path" => path.replace(PathBuf::from(value)).is_some(),
                    "--out" => out.replace(PathBuf::from(value)).is_some(),
                    _ => mes.replace(value.to_string_lossy().into_owned()).is_some(),
                };
                if slot_taken {
                    return Err(format!("`{flag}` may only be supplied once"));
                }
            }
            _ => {
                return Err(format!(
                    "unrecognized argument: {}",
                    args[index].to_string_lossy()
                ));
            }
        }
        index += 1;
    }
    let path = path.ok_or_else(|| "missing required `--path <file>`".to_owned())?;
    if let Some((name, entry, out_png)) = render_dialog {
        return run_render_dialog(
            &path,
            &name,
            entry,
            &out_png,
            dialog_variant,
            dialog_page,
            &dialog_window,
            &dialog_size,
        );
    }
    if fonts || font_png.is_some() {
        return run_font_inventory(&path, font_png, out);
    }
    if ui_atlases || atlas_png.is_some() {
        return run_ui_atlases(&path, ui_atlases, atlas_png, out, json);
    }
    let mut file = StandaloneSerialized::open(&path).map_err(|error| error.to_string())?;
    if mes.is_some() || mes_all {
        return run_mes_dump(&mut file, mes.as_deref(), json);
    }
    let inventory = file.inventory();
    let rendered = serde_json::to_string_pretty(inventory)
        .map_err(|error| format!("could not serialize inventory: {error}"))?;
    if let Some(out) = out {
        std::fs::write(
            &out,
            format!(
                "{rendered}
"
            ),
        )
        .map_err(|error| format!("could not write {}: {error}", out.display()))?;
    }
    if json {
        println!("{rendered}");
    } else {
        println!(
            "{}: {} bytes, format {}, Unity {}, {} objects",
            inventory.file_name,
            inventory.file_size,
            inventory.format_version,
            inventory.unity_version,
            inventory.object_count
        );
        for class in &inventory.classes {
            println!(
                "  class {:>4} {:<14} x{}",
                class.class_id, class.class_name, class.count
            );
            for name in class.names.iter().take(20) {
                println!("      {name}");
            }
            if class.names.len() > 20 {
                println!("      ... {} more names", class.names.len() - 20);
            }
        }
    }
    Ok(())
}

fn run_ui_atlases(
    path: &std::path::Path,
    list: bool,
    atlas_png: Option<(String, PathBuf)>,
    out: Option<PathBuf>,
    json: bool,
) -> Result<(), String> {
    use tantalus_assets::pc::ui_atlas::{
        atlas_texture_rgba, encode_png_rgba, scan_ui, scan_ui_dir,
    };
    if let Some((name, target)) = atlas_png {
        let (width, height, rgba) =
            atlas_texture_rgba(path, &name).map_err(|error| error.to_string())?;
        std::fs::write(&target, encode_png_rgba(width, height, &rgba))
            .map_err(|error| format!("could not write {}: {error}", target.display()))?;
        println!("{name}: {width}x{height} -> {}", target.display());
        if !list {
            return Ok(());
        }
    }
    let (scans, unreadable) = if path.is_dir() {
        scan_ui_dir(path).map_err(|error| error.to_string())?
    } else {
        (
            vec![scan_ui(path).map_err(|error| error.to_string())?],
            Vec::new(),
        )
    };
    let unreadable: Vec<_> = unreadable
        .into_iter()
        .map(|(file, error)| serde_json::json!({ "file": file, "error": error }))
        .collect();
    let report = serde_json::json!({
        "note": "UIAtlas/UIFont MonoBehaviours decoded from standalone Unity files; names, counts and sizes only, no pixel data.",
        "files": scans,
        "unreadable": unreadable,
    });
    let rendered = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("could not serialize UI report: {error}"))?;
    if let Some(out) = out {
        std::fs::write(
            &out,
            format!(
                "{rendered}
"
            ),
        )
        .map_err(|error| format!("could not write {}: {error}", out.display()))?;
    }
    if json {
        println!("{rendered}");
        return Ok(());
    }
    for scan in &scans {
        if scan.atlases.is_empty() && scan.fonts.is_empty() && scan.failures.is_empty() {
            continue;
        }
        println!("{}: {} MonoBehaviours", scan.file, scan.mono_behaviours);
        for atlas in &scan.atlases {
            let size = atlas.texture.as_ref().map_or("unresolved".to_owned(), |t| {
                format!("{}x{} {}", t.width, t.height, t.name)
            });
            println!(
                "  atlas {:<28} {:>4} sprites  {size}",
                atlas.name, atlas.sprite_count
            );
        }
        for font in &scan.fonts {
            let size = font.texture.as_ref().map_or("no texture".to_owned(), |t| {
                format!("{}x{} {}", t.width, t.height, t.name)
            });
            let atlas = font.atlas.as_deref().unwrap_or("-");
            println!(
                "  font  {:<28} {:>4} glyphs  atlas {atlas}  {size}",
                font.name, font.glyph_count
            );
        }
        for failure in &scan.failures {
            println!(
                "  FAIL {} {}: {}",
                failure.script, failure.path_id, failure.message
            );
        }
    }
    for item in &unreadable {
        println!("unreadable: {item}");
    }
    Ok(())
}

fn run_mes_dump(
    file: &mut tantalus_assets::pc::serialized_inventory::StandaloneSerialized,
    name: Option<&str>,
    json: bool,
) -> Result<(), String> {
    use std::collections::BTreeMap;
    use tantalus_assets::pc::mes::{MesFile, MesToken, parse_mes_bytes};
    let assets = file.text_assets().map_err(|error| error.to_string())?;
    let mut parsed_files = 0usize;
    let mut entry_total = 0usize;
    let mut tag_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    let mut layouts: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut sample_printed = false;
    for (asset_name, object) in assets {
        let selected = match name {
            Some(wanted) => asset_name == wanted,
            None => asset_name.ends_with(".mes"),
        };
        if !selected {
            continue;
        }
        let asset = file
            .read_text_asset(&object)
            .map_err(|error| error.to_string())?;
        let mes = match parse_mes_bytes(&asset.script, &asset_name) {
            Ok(mes) => mes,
            Err(error) => {
                failures.push(format!("{asset_name}: {error}"));
                continue;
            }
        };
        parsed_files += 1;
        entry_total += mes.entries.len();
        *layouts.entry(format!("{:?}", mes.layout)).or_default() += 1;
        for entry in &mes.entries {
            for token in &entry.tokens {
                if let MesToken::Tag(tag) = token {
                    *tag_counts.entry(tag.code.clone()).or_default() += 1;
                }
            }
        }
        for (raw, count) in mes.unknown_tags() {
            if name.is_none() {
                eprintln!("unknown {raw} x{count} in {asset_name}");
            }
            *unknown.entry(raw).or_default() += count;
        }
        if name.is_some() {
            println!(
                "# {asset_name} ({}) layout={:?} entries={}",
                asset.path_name,
                mes.layout,
                mes.entries.len()
            );
            for entry in &mes.entries {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string(entry).map_err(|error| error.to_string())?
                    );
                } else {
                    println!("[{}] {}", entry.id, MesFile::entry_source(entry));
                }
            }
        } else if !sample_printed && !mes.entries.is_empty() {
            sample_printed = true;
            let entry = &mes.entries[0];
            println!(
                "sample {asset_name} ({}) entry {}:",
                asset.path_name, entry.id
            );
            println!(
                "{}",
                serde_json::to_string_pretty(entry).map_err(|e| e.to_string())?
            );
        }
    }
    if name.is_none() || parsed_files == 0 {
        println!("mes files parsed: {parsed_files}");
        println!("entries: {entry_total}");
        println!("layouts: {layouts:?}");
        println!("tags: {tag_counts:?}");
        println!("unknown tags: {}", unknown.len());
        for (raw, count) in &unknown {
            println!("  {raw} x{count}");
        }
        for failure in &failures {
            println!("failure: {failure}");
        }
    }
    if parsed_files == 0 {
        return Err("no matching `.mes` TextAsset".to_owned());
    }
    Ok(())
}

struct AudioDecodeRequest<'a> {
    path: &'a std::path::Path,
    out: Option<&'a std::path::Path>,
    wav_dir: Option<&'a std::path::Path>,
    resources_assets: Option<&'a std::path::Path>,
    movie_inventory: Option<&'a std::path::Path>,
    json: bool,
}

/// Decodes every sound asset of one bundle (or of `p0data61`-`p0data63` in a directory),
/// resolves the FMV audio table and writes the audit report.
fn run_audio_decode(request: &AudioDecodeRequest<'_>) -> Result<(), String> {
    use tantalus_assets::pc::audio_decode as ad;
    let bundles: Vec<PathBuf> = if request.path.is_dir() {
        ["p0data61.bin", "p0data62.bin", "p0data63.bin"]
            .iter()
            .map(|name| request.path.join(name))
            .collect()
    } else {
        vec![request.path.to_path_buf()]
    };
    if let Some(dir) = request.wav_dir {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let mut reports = Vec::new();
    for bundle in &bundles {
        // Spot-check WAVs: the first three assets of each bundle plus the FMV059A song.
        let written = std::cell::Cell::new(0usize);
        let filter = |asset: &str| {
            let take = written.get() < 3 || asset.contains("/movie_/fmv059a");
            if take {
                written.set(written.get() + 1);
            }
            take
        };
        let report = ad::decode_audio_bundle(bundle, request.wav_dir, &filter)?;
        eprintln!(
            "{}: {} assets, {} decoded, {} failed",
            report.bundle, report.sound_assets, report.decoded_ok, report.decode_failed
        );
        reports.push(report);
    }
    // Movie names from the movie inventory, then the resource metadata.
    let inventory_path = request
        .movie_inventory
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("audit/movie-inventory.json"));
    let mut movies = Vec::new();
    if let Ok(text) = std::fs::read_to_string(&inventory_path) {
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        for file in value["files"].as_array().into_iter().flatten() {
            if let Some(name) = file["file"].as_str() {
                movies.push(name.split('.').next().unwrap_or(name).to_owned());
            }
        }
    }
    let mut notes = Vec::new();
    let resources = request.resources_assets.map(PathBuf::from).or_else(|| {
        let dir = if request.path.is_file() {
            request.path.parent()?
        } else {
            request.path
        };
        let derived = dir.parent()?.join("x64/FF9_Data/resources.assets");
        derived.is_file().then_some(derived)
    });
    let metadata = match resources {
        Some(file) => match ad::read_text_asset(&file, "MovieAudioMetaData.txt")
            .and_then(|bytes| ad::parse_movie_audio_metadata(&bytes))
        {
            Ok(map) => Some(map),
            Err(e) => {
                notes.push(format!("MovieAudioMetaData.txt not read: {e}"));
                None
            }
        },
        None => {
            notes.push("MovieAudioMetaData.txt not read: resources.assets not found".to_owned());
            None
        }
    };
    if movies.is_empty() {
        notes
            .push("no movie inventory: only explicit GetMovieSoundIndex cases resolved".to_owned());
    }
    let rows = ad::resolve_movie_audio(&movies, metadata.as_ref(), &reports);
    let unresolved: Vec<_> = rows.iter().filter(|r| !r.resolved).collect();
    let (ok, failed) = reports
        .iter()
        .fold((0, 0), |(a, b), r| (a + r.decoded_ok, b + r.decode_failed));
    let document = serde_json::json!({
        "description": "Vorbis decode of AKB2 sound assets plus the FMV-to-sound table, from tantalusctl assets inspect-bundle --audio-decode",
        "decoder": "lewton 0.10.2 (MIT OR Apache-2.0), pure Rust",
        "totals": {
            "sound_assets": reports.iter().map(|r| r.sound_assets).sum::<usize>(),
            "decoded_ok": ok,
            "decode_failed": failed,
            "movie_rows": rows.len(),
            "movie_rows_unresolved": unresolved.len(),
        },
        "notes": notes,
        "bundles": reports,
        "movie_audio": rows,
    });
    let text = serde_json::to_string_pretty(&document).map_err(|e| e.to_string())?;
    let out = request
        .out
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("audit/audio-decode.json"));
    std::fs::write(&out, format!("{text}\n"))
        .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    if request.json {
        println!("{text}");
    } else {
        println!(
            "decoded {ok} ok, {failed} failed; {} movie rows, {} unresolved; wrote {}",
            rows.len(),
            unresolved.len(),
            out.display()
        );
        for row in unresolved {
            println!(
                "UNRESOLVED {} ({}): {}",
                row.movie,
                row.resource_id,
                row.unresolved_reason.as_deref().unwrap_or("")
            );
        }
    }
    Ok(())
}

/// Classifies every sound asset in a bundle and prints AKB2/Ogg statistics.
fn run_audio_all(path: &std::path::Path, json: bool) -> Result<(), String> {
    let report = tantalus_assets::pc::akb2::inspect_audio_bundle(path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return Ok(());
    }
    println!("{}: {} sound assets", report.bundle, report.sound_assets);
    println!("container types: {:?}", report.by_container_type);
    println!("payload magic: {:?}", report.by_magic);
    println!(
        "AKB headers parsed {} (payload in range {}); sample rate {:?}..{:?}; looped {} ({} alternate); header samples {}, stream samples {}",
        report.akb_parsed,
        report.akb_payload_in_range,
        report.min_sample_rate,
        report.max_sample_rate,
        report.looped,
        report.with_alternate_loop,
        report.total_samples,
        report.total_granule_samples
    );
    println!(
        "codecs: {:?}; ogg pages {}; sample_count != last granule: {}",
        report.codecs, report.ogg_pages, report.sample_count_mismatches
    );
    for failure in &report.failures {
        println!("FAIL {}: {}", failure.path, failure.reason);
    }
    Ok(())
}

fn run_world_map(
    bundle: &tantalus_assets::pc::unity_raw::UnityRawBundle,
    path: &std::path::Path,
    json: bool,
    audit: Option<&std::path::Path>,
) -> Result<(), String> {
    let source = path.to_string_lossy();
    let bytes = bundle
        .read_entry(0, 600 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    let serialized = tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &source)
        .map_err(|error| error.to_string())?;
    let report = tantalus_assets::pc::world_map::unity::analyze_world_map(&serialized, &source)
        .map_err(|error| error.to_string())?;
    if let Some(target) = audit {
        let text = serde_json::to_string_pretty(&report.audit_json())
            .map_err(|error| error.to_string())?;
        std::fs::write(
            target,
            format!(
                "{text}
"
            ),
        )
        .map_err(|error| format!("cannot write {}: {error}", target.display()))?;
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return Ok(());
    }
    println!(
        "{} container entries ({} distinct paths): {} classified, {} unclassified",
        report.container_entries,
        report.distinct_container_paths,
        report.classified_entries,
        report.unclassified_entries
    );
    for group in &report.groups {
        println!(
            "  {:<36} class {:>3} {:>5}",
            group.group, group.class_id, group.count
        );
    }
    println!(
        "block meshes: {} decoded, {} failed, {} vertices, {} triangles",
        report.block_meshes_decoded,
        report.block_meshes_failed,
        report.block_mesh_vertices,
        report.block_mesh_triangles
    );
    for disc in &report.discs {
        println!(
            "  disc {}: {} cells with meshes ({} meshes, {} vertices, {} triangles), {} prefabs, {}/{} prefab slots match their block, {} meshes unreferenced, grid x {}..={} y {}..={}",
            disc.disc,
            disc.cells_with_meshes,
            disc.block_meshes,
            disc.vertices,
            disc.triangles,
            disc.block_prefabs,
            disc.prefab_slots_matching_block,
            disc.prefab_slots,
            disc.meshes_not_in_any_prefab,
            disc.grid_min_x,
            disc.grid_max_x,
            disc.grid_min_y,
            disc.grid_max_y
        );
    }
    println!(
        "other objects: {}/{} materials, {}/{} textures, {} render textures, {}/{} other meshes",
        report.objects.materials_decoded,
        report.objects.materials,
        report.objects.textures_decoded,
        report.objects.textures,
        report.objects.render_textures,
        report.objects.other_meshes_decoded,
        report.objects.other_meshes
    );
    println!(
        "wmap binaries: {} assets, {} bytes: {} parsed, {} zero padding, {} raw named, {} unexplained; {} with failures",
        report.wmap_assets.len(),
        report.wmap_bytes_total,
        report.wmap_bytes_parsed,
        report.wmap_bytes_zero_padding,
        report.wmap_bytes_raw_named,
        report.wmap_bytes_unexplained,
        report.wmap_failures
    );
    for asset in &report.wmap_assets {
        println!(
            "  {:<12} {:>8} B  parsed {:>7} pad {:>6} raw {:>8} unexplained {:>5}  {}",
            asset.kind,
            asset.bytes,
            asset.account.parsed,
            asset.account.zero_padding,
            asset.account.raw_named,
            asset.account.unexplained,
            asset
                .path
                .trim_start_matches("assets/resources/worldmap/wmap/")
        );
        for failure in &asset.failures {
            println!("      failure: {failure}");
        }
    }
    for failure in report.failures.iter().take(20) {
        println!("failure: {failure}");
    }
    Ok(())
}

/// `assets inspect-movie --dir <dir> --decode-frames <name> <n|all> [--png <dir>] [--out <json>]`.
/// Decodes Theora frames, checks the decoded packet count against the Ogg
/// granule position, hashes every frame and writes `audit/movie-decode.json`
/// (or `--out`). PNG output is for local inspection and is never committed.
fn run_movie_decode(
    dir: &std::path::Path,
    name: &str,
    limit: Option<u64>,
    out: Option<PathBuf>,
    png_dir: Option<&std::path::Path>,
    json: bool,
) -> Result<(), String> {
    use tantalus_assets::install::ImportError;
    use tantalus_assets::pc::theora::decode_movie;
    use tantalus_assets::pc::ui_atlas::encode_png_rgba;
    let stem = name.strip_suffix(".bytes").unwrap_or(name);
    let path = dir.join(format!("{stem}.bytes"));
    let file = std::fs::File::open(&path)
        .map_err(|e| format!("could not open {}: {e}", path.display()))?;
    let file_size = file.metadata().map_err(|e| e.to_string())?.len();
    if let Some(png_dir) = png_dir {
        std::fs::create_dir_all(png_dir)
            .map_err(|e| format!("could not create {}: {e}", png_dir.display()))?;
    }
    let mut frame_hashes: Vec<String> = Vec::new();
    let mut frame0 = serde_json::Value::Null;
    let mut last_png: Option<(u64, u32, u32, Vec<u8>)> = None;
    let started = std::time::Instant::now();
    let summary = decode_movie(std::io::BufReader::new(file), limit, |index, frame| {
        let mut hasher = Sha256::new();
        hasher.update(&frame.y);
        hasher.update(&frame.cb);
        hasher.update(&frame.cr);
        frame_hashes.push(format!("{:x}", hasher.finalize()));
        if index == 0 {
            frame0 = serde_json::json!({
                "width": frame.width,
                "height": frame.height,
                "y_sha256": format!("{:x}", Sha256::digest(&frame.y)),
                "cb_sha256": format!("{:x}", Sha256::digest(&frame.cb)),
                "cr_sha256": format!("{:x}", Sha256::digest(&frame.cr)),
            });
        }
        if let Some(png_dir) = png_dir {
            let rgba = frame.to_rgba();
            if index == 0 {
                let target = png_dir.join(format!("{stem}_f{index:05}.png"));
                std::fs::write(&target, encode_png_rgba(frame.width, frame.height, &rgba))
                    .map_err(|e| ImportError::invalid(target.display(), e))?;
            }
            last_png = Some((index, frame.width, frame.height, rgba));
        }
        Ok(())
    })
    .map_err(|e| format!("{}: {e}", path.display()))?;
    let elapsed = started.elapsed().as_secs_f64();
    if let (Some(png_dir), Some((index, w, h, rgba))) = (png_dir, last_png) {
        let target = png_dir.join(format!("{stem}_f{index:05}.png"));
        std::fs::write(&target, encode_png_rgba(w, h, &rgba))
            .map_err(|e| format!("could not write {}: {e}", target.display()))?;
    }
    let count_check = match (summary.complete, summary.granule_frame_count) {
        (true, Some(declared)) => serde_json::json!({
            "checked": true,
            "granule_declared_frames": declared,
            "decoded_packets": summary.packets_decoded,
            "equal": declared == summary.packets_decoded,
        }),
        _ => serde_json::json!({
            "checked": false,
            "reason": "decode stopped before the end of the stream or no data page granule was seen",
        }),
    };
    let chain = {
        let mut hasher = Sha256::new();
        for hash in &frame_hashes {
            hasher.update(hash.as_bytes());
        }
        format!("{:x}", hasher.finalize())
    };
    let fps = if elapsed > 0.0 {
        (summary.packets_decoded as f64 / elapsed * 10.0).round() / 10.0
    } else {
        0.0
    };
    let document = serde_json::json!({
        "file": format!("{stem}.bytes"),
        "file_size": file_size,
        "decoder": "oxideav-theora 0.0.12 (pure Rust, MIT) via tantalus_assets::pc::theora",
        "picture": format!("{}x{}", summary.params.picture_width, summary.params.picture_height),
        "pixel_format": summary.params.pixel_format,
        "fps": format!("{}/{}", summary.params.fps_numerator, summary.params.fps_denominator),
        "keyframe_granule_shift": summary.params.keyframe_granule_shift,
        "requested_frames": limit,
        "decoded_packets": summary.packets_decoded,
        "empty_duplicate_packets": summary.empty_packets,
        "complete": summary.complete,
        "last_granule": summary.last_granule,
        "count_check": count_check,
        "decode_seconds": (elapsed * 1000.0).round() / 1000.0,
        "decode_frames_per_second": fps,
        "frame0": frame0,
        "frame_hash_algorithm": "sha256 over the cropped top-down Y, Cb, Cr planes concatenated",
        "frame_hash_chain_sha256": chain,
        "frame_hashes": frame_hashes,
    });
    let text = serde_json::to_string_pretty(&document)
        .map_err(|e| format!("could not serialize movie decode report: {e}"))?;
    let out = out.unwrap_or_else(|| PathBuf::from("audit/movie-decode.json"));
    std::fs::write(&out, format!("{text}\n"))
        .map_err(|e| format!("could not write {}: {e}", out.display()))?;
    if json {
        println!("{text}");
    } else {
        println!(
            "{stem}: decoded {} packets ({} empty) in {elapsed:.2}s, count check {}, wrote {}",
            summary.packets_decoded,
            summary.empty_packets,
            document["count_check"]["equal"],
            out.display()
        );
    }
    Ok(())
}

fn run_assets_inspect_movie(args: Vec<OsString>) -> Result<(), String> {
    use std::collections::BTreeMap;
    use tantalus_assets::pc::ogg::{StreamCodec, scan_ogg_file};
    let mut dir = None;
    let mut out = None;
    let mut json = false;
    let mut verify_crc = false;
    let mut decode_frames: Option<(String, Option<u64>)> = None;
    let mut png_dir: Option<PathBuf> = None;
    let mut index = 2;
    while index < args.len() {
        match args[index].to_str() {
            Some("--dir") => {
                index += 1;
                dir = Some(PathBuf::from(
                    args.get(index).ok_or("`--dir` requires a path")?,
                ));
            }
            Some("--out") => {
                index += 1;
                out = Some(PathBuf::from(
                    args.get(index).ok_or("`--out` requires a path")?,
                ));
            }
            Some("--json") => json = true,
            Some("--verify-crc") => verify_crc = true,
            Some("--decode-frames") => {
                let name = args
                    .get(index + 1)
                    .ok_or("`--decode-frames` requires a name")?;
                let count = args
                    .get(index + 2)
                    .and_then(|v| v.to_str())
                    .ok_or("`--decode-frames` requires a frame count or `all`")?;
                let limit = if count == "all" {
                    None
                } else {
                    Some(count.parse::<u64>().map_err(|_| {
                        format!("bad frame count `{count}`: expected a number or `all`")
                    })?)
                };
                decode_frames = Some((name.to_string_lossy().into_owned(), limit));
                index += 2;
            }
            Some("--png") => {
                index += 1;
                png_dir = Some(PathBuf::from(
                    args.get(index).ok_or("`--png` requires a directory")?,
                ));
            }
            _ => {
                return Err(format!(
                    "unrecognized argument: {}",
                    args[index].to_string_lossy()
                ));
            }
        }
        index += 1;
    }
    let dir = dir.ok_or_else(|| "missing required `--dir <ma dir>`".to_owned())?;
    if let Some((name, limit)) = decode_frames {
        return run_movie_decode(&dir, &name, limit, out, png_dir.as_deref(), json);
    }
    if png_dir.is_some() {
        return Err("`--png` needs `--decode-frames <name> <n>`".to_owned());
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("could not read {}: {e}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "bytes"))
        .collect();
    paths.sort();

    let mut files = Vec::new();
    let mut totals_codecs: BTreeMap<String, u64> = BTreeMap::new();
    let mut audio_formats: BTreeMap<String, u64> = BTreeMap::new();
    let mut resolutions: BTreeMap<String, u64> = BTreeMap::new();
    let mut frame_rates: BTreeMap<String, u64> = BTreeMap::new();
    let (mut theora_files, mut audio_files, mut parse_failures, mut crc_failures) = (0, 0, 0, 0);
    let (mut total_bytes, mut total_pages, mut seq_gaps) = (0_u64, 0_u64, 0_u64);
    let mut video_seconds = 0.0_f64;
    let mut fmv_without_audio = Vec::new();
    for path in &paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match scan_ogg_file(path, verify_crc) {
            Err(error) => {
                parse_failures += 1;
                files.push(serde_json::json!({"file": name, "error": error.to_string()}));
            }
            Ok(report) => {
                total_bytes += report.file_size;
                total_pages += report.page_count;
                seq_gaps += report.sequence_gaps;
                crc_failures += report.crc_failures.len();
                if report.parse_error.is_some() {
                    parse_failures += 1;
                }
                let mut has_theora = false;
                let mut has_audio = false;
                for stream in &report.streams {
                    *totals_codecs
                        .entry(stream.codec.name().to_owned())
                        .or_default() += 1;
                    match &stream.codec {
                        StreamCodec::Theora(t) => {
                            if !has_theora {
                                has_theora = true;
                                theora_files += 1;
                                video_seconds += stream.duration_seconds.unwrap_or(0.0);
                            }
                            *resolutions
                                .entry(format!("{}x{}", t.picture_width, t.picture_height))
                                .or_default() += 1;
                            *frame_rates
                                .entry(format!("{}/{}", t.fps_numerator, t.fps_denominator))
                                .or_default() += 1;
                        }
                        StreamCodec::Vorbis(v) => {
                            has_audio = true;
                            *audio_formats
                                .entry(format!("vorbis {}ch {}Hz", v.channels, v.sample_rate))
                                .or_default() += 1;
                        }
                        StreamCodec::Opus { channels, .. } => {
                            has_audio = true;
                            *audio_formats
                                .entry(format!("opus {channels}ch"))
                                .or_default() += 1;
                        }
                        StreamCodec::Flac => {
                            has_audio = true;
                            *audio_formats.entry("flac".to_owned()).or_default() += 1;
                        }
                        StreamCodec::Other { .. } => {}
                    }
                }
                if has_audio {
                    audio_files += 1;
                } else if name.starts_with("FMV") {
                    fmv_without_audio.push(name.clone());
                }
                files.push(serde_json::json!({"file": name, "report": report}));
            }
        }
    }
    let totals = serde_json::json!({
        "files": paths.len(),
        "total_bytes": total_bytes,
        "total_pages": total_pages,
        "crc_verified": verify_crc,
        "crc_failures": crc_failures,
        "sequence_gaps": seq_gaps,
        "parse_failures": parse_failures,
        "files_with_theora": theora_files,
        "files_with_audio_stream": audio_files,
        "fmv_files_without_audio": fmv_without_audio,
        "streams_by_codec": totals_codecs,
        "audio_formats": audio_formats,
        "video_resolutions": resolutions,
        "video_frame_rates": frame_rates,
        "total_video_duration_seconds": video_seconds,
    });
    let document = serde_json::json!({"totals": totals, "files": files});
    let text = serde_json::to_string_pretty(&document)
        .map_err(|e| format!("could not serialize movie inventory: {e}"))?;
    if let Some(out) = &out {
        std::fs::write(out, format!("{text}\n"))
            .map_err(|e| format!("could not write {}: {e}", out.display()))?;
    }
    if json {
        println!("{text}");
        return Ok(());
    }
    for entry in &files {
        let name = entry["file"].as_str().unwrap_or("?");
        if let Some(error) = entry.get("error") {
            println!("{name}: ERROR {error}");
            continue;
        }
        let r = &entry["report"];
        let mut line = format!(
            "{name}: {} bytes, {} pages, {} crc failures",
            r["file_size"],
            r["page_count"],
            r["crc_failures"].as_array().map_or(0, Vec::len)
        );
        for s in r["streams"].as_array().into_iter().flatten() {
            let c = &s["codec"];
            line.push_str(&format!(
                " | {} {}x{} {}/{} fps {}ch {}Hz {:.1}s",
                c["codec"].as_str().unwrap_or("?"),
                c["picture_width"],
                c["picture_height"],
                c["fps_numerator"],
                c["fps_denominator"],
                c["channels"],
                c["sample_rate"],
                s["duration_seconds"].as_f64().unwrap_or(0.0)
            ));
        }
        if let Some(e) = r["parse_error"].as_str() {
            line.push_str(&format!(" | PARSE ERROR {e}"));
        }
        println!("{line}");
    }
    println!("{totals}");
    Ok(())
}
