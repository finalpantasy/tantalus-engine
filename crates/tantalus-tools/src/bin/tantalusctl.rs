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
    let mut texture = None;
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
    if let Some(name) = texture {
        let bytes = bundle
            .read_entry(0, 300 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let serialized =
            tantalus_assets::pc::serialized::SerializedFile::parse(&bytes, &path.to_string_lossy())
                .map_err(|error| error.to_string())?;
        let mut selected = None;
        for (index, object) in serialized.report().objects.iter().enumerate() {
            if object.class_id != 28 {
                continue;
            }
            let texture = tantalus_assets::pc::texture::Texture2D::parse(
                serialized
                    .object_data(index)
                    .map_err(|error| error.to_string())?,
                &name,
            )
            .map_err(|error| error.to_string())?;
            if texture.name == name {
                selected = Some((object.path_id, texture));
                break;
            }
        }
        let (path_id, texture) =
            selected.ok_or_else(|| format!("Texture2D `{name}` was not found"))?;
        let rgba = texture.rgba_bytes().map_err(|error| error.to_string())?;
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

fn usage() -> &'static str {
    "expected `install inspect --root <path> [--json]`, `assets import --plan <path> [--json]`, or `assets inspect-bundle --path <path> [--objects] [--text-asset <name>] [--texture <name>] [--json]`"
}

fn print_help() {
    println!(
        "tantalusctl\n\nUSAGE:\n    tantalusctl install inspect --root <path> [--json]\n    tantalusctl assets import --plan <path> [--json]\n    tantalusctl assets inspect-bundle --path <path> [--objects] [--text-asset <name>] [--texture <name>] [--json]\n\nInspect installations and UnityRaw bundles without modifying them, or import validated local asset packs."
    );
}
