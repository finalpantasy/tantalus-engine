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
            "tantalus-client capture --mode <2d|3d|hybrid> --output <png>\ntantalus-client capture-field --bundle <unityraw> --field <field> --output <png> [--size WxH]\ntantalus-client view-field --bundle <unityraw> --field <name> [--scale N]"
        );
        return ExitCode::SUCCESS;
    }
    match args.first().and_then(|arg| arg.to_str()) {
        Some("capture") => run_synthetic_capture(&args[1..]),
        Some("capture-field") => run_field_capture(&args[1..]),
        Some("view-field") => run_field_view(&args[1..]),
        _ => {
            eprintln!("expected `capture`, `capture-field` or `view-field` (see --help)");
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
