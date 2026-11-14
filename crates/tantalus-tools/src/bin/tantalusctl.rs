use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use tantalus_assets::inspect_install;

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
    if args.len() < 2 || args[0] != "install" || args[1] != "inspect" {
        return Err("expected `install inspect --root <path> [--json]`".to_owned());
    }

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

fn print_help() {
    println!(
        "tantalusctl\n\nUSAGE:\n    tantalusctl install inspect --root <path> [--json]\n\nInspects a PC installation without modifying it."
    );
}
