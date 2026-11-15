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
        (Some("assets"), Some("extract-resource")) => run_assets_extract_resource(args),
        (Some("assets"), Some("extract-text")) => run_assets_extract_text(args),
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
    let mut atan_table = None;
    let mut navigation_bundle = None;
    let mut navigation_bgi = None;
    let mut ticks = 64u64;
    let mut probe = false;
    let mut inventory = false;
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
            Some("--json") => json = true,
            Some("--objects") => objects = true,
            Some("--probe") => probe = true,
            Some("--inventory") => inventory = true,
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
        let mut operations = 0u64;
        let mut error = None;
        let mut ticks_run = 0u64;
        let mut spawned: Vec<(tantalus_contracts::EntityId, u64)> = Vec::new();
        let mut unspawned_requests = 0usize;
        for _ in 0..ticks {
            match director.step() {
                Ok(step) => {
                    operations += step.operations;
                    ticks_run += 1;
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
                serde_json::json!({
                    "entry": entity.get(),
                    "pc": pc,
                    "status": format!("{status:?}"),
                    "wait": wait.map(|wait| format!("{wait:?}")),
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

fn usage() -> &'static str {
    "expected `install inspect --root <path> [--json]`, `assets import --plan <path> [--json]`, `assets inspect-bundle --path <path> [--objects] [--text-asset <name>] [--inventory] [--event <name> [--probe]] [--texture <name>] [--texture-id <path-id>] [--rgba-output <path>] [--bgi <name>] [--bgs <name>] [--field <name>] [--atlas-for <field>] [--json]`, or `scenario run --case <path> [--bundle <path>] [--json]`"
}

fn print_help() {
    println!(
        "tantalusctl\n\nUSAGE:\n    tantalusctl install inspect --root <path> [--json]\n    tantalusctl assets import --plan <path> [--json]\n    tantalusctl assets inspect-bundle --path <path> [--objects] [--text-asset <name>] [--inventory] [--event <name> [--probe]] [--texture <name>] [--texture-id <path-id>] [--rgba-output <path>] [--bgi <name>] [--bgs <name>] [--field <name>] [--atlas-for <field>] [--json]\n    tantalusctl scenario run --case <path> [--bundle <path>] [--json]\n\nInspect installations and UnityRaw bundles, import validated local asset packs, or run typed headless scenarios."
    );
}
