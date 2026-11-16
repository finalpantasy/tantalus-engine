use std::path::PathBuf;
use std::process::ExitCode;
use tantalus_assets::pc::bundled_field::BundledField;
use tantalus_client::capture::{
    CaptureConfig, capture_field_with, capture_snapshot, synthetic_snapshot,
};
use tantalus_contracts::ScenePresentationMode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "tantalus-client capture --mode <2d|3d|hybrid> --output <png>\ntantalus-client capture-field --bundle <unityraw> --field <field> --output <png> [--size WxH]\ntantalus-client view-field --bundle <unityraw> --field <name> [--scale N]\ntantalus-client view-track --assets <StreamingAssets> --track <NNN_MAP.jsonl> [--scale N]\ntantalus-client capture-track --assets <StreamingAssets> --track <file> --tick T --output <png> [--size WxH]"
        );
        return ExitCode::SUCCESS;
    }
    match args.first().and_then(|arg| arg.to_str()) {
        Some("capture") => run_synthetic_capture(&args[1..]),
        Some("capture-field") => run_field_capture(&args[1..]),
        Some("view-field") => run_field_view(&args[1..]),
        Some("view-track") => run_track_view(&args[1..]),
        Some("capture-track") => run_track_capture(&args[1..]),
        _ => {
            eprintln!(
                "expected `capture`, `capture-field` `view-field`, `view-track` or `capture-track` (see --help)"
            );
            ExitCode::FAILURE
        }
    }
}

fn run_synthetic_capture(args: &[std::ffi::OsString]) -> ExitCode {
    let mut mode = None;
    let mut output = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].to_str() {
            Some("--mode") => {
                index += 1;
                mode = args.get(index).and_then(|value| match value.to_str()? {
                    "2d" => Some(ScenePresentationMode::Layered2d),
                    "3d" => Some(ScenePresentationMode::Full3d),
                    "hybrid" => Some(ScenePresentationMode::Hybrid),
                    _ => None,
                });
            }
            Some("--output") => {
                index += 1;
                output = args.get(index).map(PathBuf::from);
            }
            _ => {
                eprintln!("unknown argument: {}", args[index].to_string_lossy());
                return ExitCode::FAILURE;
            }
        }
        index += 1;
    }

    let (Some(mode), Some(output)) = (mode, output) else {
        eprintln!("capture requires --mode and --output");
        return ExitCode::FAILURE;
    };
    match capture_snapshot(&synthetic_snapshot(mode), &output, CaptureConfig::default()) {
        Ok(()) => {
            println!("saved headless capture to {}", output.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("capture failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_field_capture(args: &[std::ffi::OsString]) -> ExitCode {
    let mut bundle = None;
    let mut field = None;
    let mut output = None;
    let mut size = None;
    let mut walkmesh = false;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--walkmesh" {
            walkmesh = true;
            index += 1;
            continue;
        }
        let destination = match args[index].to_str() {
            Some("--bundle") => &mut bundle,
            Some("--field") => &mut field,
            Some("--output") => &mut output,
            Some("--size") => &mut size,
            _ => {
                eprintln!("unknown argument: {}", args[index].to_string_lossy());
                return ExitCode::FAILURE;
            }
        };
        index += 1;
        let Some(value) = args.get(index) else {
            eprintln!("capture-field option requires a value");
            return ExitCode::FAILURE;
        };
        if destination.replace(PathBuf::from(value)).is_some() {
            eprintln!("capture-field options may only be supplied once");
            return ExitCode::FAILURE;
        }
        index += 1;
    }

    let (Some(bundle), Some(field), Some(output)) = (bundle, field, output) else {
        eprintln!("capture-field requires --bundle, --field, and --output");
        return ExitCode::FAILURE;
    };
    let Some(field_name) = field.to_str() else {
        eprintln!("capture-field field name must be UTF-8");
        return ExitCode::FAILURE;
    };
    let loaded = match BundledField::load(&bundle, field_name) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("field loading failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (width, height) = match size.as_deref().map(parse_size) {
        None => (320, 240),
        Some(Some(size)) => size,
        Some(None) => {
            eprintln!("--size must be WxH, for example 960x720");
            return ExitCode::FAILURE;
        }
    };
    let config = CaptureConfig {
        width,
        height,
        ..CaptureConfig::default()
    };
    match capture_field_with(&loaded, &output, config, walkmesh) {
        Ok(()) => {
            println!("saved headless field capture to {}", output.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("field capture failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_size(value: &std::path::Path) -> Option<(u32, u32)> {
    let (width, height) = value.to_str()?.split_once('x')?;
    let (width, height) = (width.parse().ok()?, height.parse().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

fn run_field_view(args: &[std::ffi::OsString]) -> ExitCode {
    let mut bundle = None;
    let mut field = None;
    let mut scale = 3;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].to_str();
        let Some(value) = args.get(index + 1) else {
            eprintln!("view-field option requires a value");
            return ExitCode::FAILURE;
        };
        match flag {
            Some("--bundle") => bundle = Some(PathBuf::from(value)),
            Some("--field") => field = value.to_str().map(str::to_owned),
            Some("--scale") => match value.to_str().and_then(|v| v.parse().ok()) {
                Some(value) => scale = value,
                None => {
                    eprintln!("--scale must be a whole number");
                    return ExitCode::FAILURE;
                }
            },
            _ => {
                eprintln!("unknown argument: {}", args[index].to_string_lossy());
                return ExitCode::FAILURE;
            }
        }
        index += 2;
    }
    let (Some(bundle), Some(field)) = (bundle, field) else {
        eprintln!("view-field requires --bundle and --field");
        return ExitCode::FAILURE;
    };
    let loaded = match BundledField::load(&bundle, &field) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("field loading failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    match tantalus_client::window::view_field(&loaded, scale) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("field view failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `--flag value` pairs into a map; `None` (after printing why) on a flag
/// outside `allowed` or a flag with no value.
fn flag_values(
    args: &[std::ffi::OsString],
    allowed: &[&str],
) -> Option<std::collections::BTreeMap<String, String>> {
    let mut values = std::collections::BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].to_string_lossy().into_owned();
        if !allowed.contains(&flag.as_str()) {
            eprintln!("unknown argument: {flag}");
            return None;
        }
        let Some(value) = args.get(index + 1) else {
            eprintln!("{flag} requires a value");
            return None;
        };
        values.insert(flag, value.to_string_lossy().into_owned());
        index += 2;
    }
    Some(values)
}

type TrackAndField = (
    tantalus_client::track::ActorTrack,
    tantalus_assets::pc::bundled_field::BundledField,
);

fn load_track_and_field(
    values: &std::collections::BTreeMap<String, String>,
) -> Result<TrackAndField, String> {
    let (Some(dir), Some(track)) = (values.get("--assets"), values.get("--track")) else {
        return Err("requires --assets <StreamingAssets dir> and --track <file>".to_owned());
    };
    let track = tantalus_client::track::ActorTrack::load(std::path::Path::new(track))
        .map_err(|error| error.to_string())?;
    let field = tantalus_client::window::find_field(std::path::Path::new(dir), &track.field)
        .map_err(|error| error.to_string())?;
    Ok((track, field))
}

fn run_track_view(args: &[std::ffi::OsString]) -> ExitCode {
    let Some(values) = flag_values(args, &["--assets", "--track", "--scale"]) else {
        return ExitCode::FAILURE;
    };
    let scale = values
        .get("--scale")
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);
    let result = load_track_and_field(&values).and_then(|(track, field)| {
        tantalus_client::window::view_track(&field, track, scale).map_err(|error| error.to_string())
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("view-track failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_track_capture(args: &[std::ffi::OsString]) -> ExitCode {
    let allowed = ["--assets", "--track", "--tick", "--output", "--size"];
    let Some(values) = flag_values(args, &allowed) else {
        return ExitCode::FAILURE;
    };
    let Some(output) = values.get("--output").map(PathBuf::from) else {
        eprintln!("capture-track requires --output");
        return ExitCode::FAILURE;
    };
    let tick: u64 = values
        .get("--tick")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let (width, height) = match values.get("--size") {
        None => (320, 240),
        Some(size) => match parse_size(std::path::Path::new(size)) {
            Some(size) => size,
            None => {
                eprintln!("--size must be WxH, for example 960x720");
                return ExitCode::FAILURE;
            }
        },
    };
    let result = load_track_and_field(&values).and_then(|(track, field)| {
        let frame = track
            .frames
            .iter()
            .rev()
            .find(|frame| frame.tick <= tick)
            .or(track.frames.first())
            .cloned()
            .ok_or("track has no frames")?;
        let placed = track.placed_actors();
        let mut drawn = 0;
        tantalus_client::capture::capture_field_scene(
            &field,
            &output,
            CaptureConfig {
                width,
                height,
                ..CaptureConfig::default()
            },
            |world| {
                drawn = tantalus_client::track::spawn_frame_markers(
                    world,
                    &field.scene.camera,
                    &frame,
                    &placed,
                );
            },
        )
        .map_err(|error| error.to_string())?;
        Ok((frame.tick, drawn))
    });
    match result {
        Ok((tick, drawn)) => {
            println!(
                "saved tick {tick} with {drawn} actors to {}",
                output.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("capture-track failed: {error}");
            ExitCode::FAILURE
        }
    }
}
