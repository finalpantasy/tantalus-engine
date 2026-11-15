use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::PathBuf;
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
        (Some("assets"), Some("import")) => run_assets_import(args),
        (Some("assets"), Some("inspect-bundle")) => run_assets_inspect_bundle(args),
        (Some("scenario"), Some("run")) => run_scenario(args),
        _ => Err(usage().to_owned()),
    }
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

fn run_assets_inspect_bundle(args: Vec<OsString>) -> Result<(), String> {
    let mut path = None;
    let mut json = false;
    let mut objects = false;
    let mut text_asset = None;
    let mut event = None;
    let mut texture = None;
    let mut texture_id = None;
    let mut rgba_output = None;
    let mut bgi = None;
    let mut bgs = None;
    let mut field = None;
    let mut atlas_for = None;
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
            Some("--json") => json = true,
            Some("--objects") => objects = true,
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
    let bundle = tantalus_assets::pc::unity_raw::UnityRawBundle::open(&path)
        .map_err(|error| error.to_string())?;
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
            let mut first_decode_error = None;
            for entry in &binary.entries {
                match binary.decode_entry(entry.index) {
                    Ok(_) => decoded_entries += 1,
                    Err(error) if first_decode_error.is_none() => {
                        first_decode_error = Some(serde_json::json!({
                            "entry": entry.index,
                            "kind": format!("{:?}", error.kind),
                            "offset": error.offset,
                            "opcode": error.opcode,
                            "message": error.message,
                        }));
                    }
                    Err(_) => {}
                }
            }
            candidates.push(serde_json::json!({
                "path_id": object.path_id,
                "asset_bytes": asset.script.len(),
                "version": binary.version,
                "entries": binary.entries.len(),
                "decoded_entries": decoded_entries,
                "first_decode_error": first_decode_error,
                "script_bytes": script_bytes,
                "script_sha256": format!("{:x}", digest.finalize()),
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

fn usage() -> &'static str {
    "expected `install inspect --root <path> [--json]`, `assets import --plan <path> [--json]`, `assets inspect-bundle --path <path> [--objects] [--text-asset <name>] [--event <name>] [--texture <name>] [--texture-id <path-id>] [--rgba-output <path>] [--bgi <name>] [--bgs <name>] [--field <name>] [--atlas-for <field>] [--json]`, or `scenario run --case <path> [--bundle <path>] [--json]`"
}

fn print_help() {
    println!(
        "tantalusctl\n\nUSAGE:\n    tantalusctl install inspect --root <path> [--json]\n    tantalusctl assets import --plan <path> [--json]\n    tantalusctl assets inspect-bundle --path <path> [--objects] [--text-asset <name>] [--event <name>] [--texture <name>] [--texture-id <path-id>] [--rgba-output <path>] [--bgi <name>] [--bgs <name>] [--field <name>] [--atlas-for <field>] [--json]\n    tantalusctl scenario run --case <path> [--bundle <path>] [--json]\n\nInspect installations and UnityRaw bundles, import validated local asset packs, or run typed headless scenarios."
    );
}
