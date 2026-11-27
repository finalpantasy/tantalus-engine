mod battle_capture;

use std::path::PathBuf;
use std::process::ExitCode;
use tantalus_assets::pc::bundled_field::BundledField;
use tantalus_client::capture::{
    CaptureConfig, capture_field_with, capture_snapshot, synthetic_snapshot,
};
use tantalus_contracts::ScenePresentationMode;

fn main() -> ExitCode {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    // `--aspect 16:9|16:10|21:9|4:3|auto` (or `aspect = "..."` in `tantalus.toml`).
    match tantalus_client::widescreen::take_aspect_flag(&mut args) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    }
    // Opt-in Memoria `KeepRestTimeInBattle` (default off, vanilla).
    let (keep_rest, args) = take_switch(&args, "--keep-rest-time");
    let env_on = std::env::var("TANTALUS_KEEP_REST_TIME").is_ok_and(|v| v == "1");
    tantalus_legacy::battle::machine::set_keep_rest_time_default(keep_rest || env_on);
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "tantalus-client capture --mode <2d|3d|hybrid> --output <png>\ntantalus-client capture-field --bundle <unityraw> --field <field> --output <png> [--size WxH]\ntantalus-client view-field --bundle <unityraw> --field <name> [--scale N]\ntantalus-client view-track --assets <StreamingAssets> --track <NNN_MAP.jsonl> [--scale N]\ntantalus-client capture-track --assets <StreamingAssets> --track <file> --tick T --output <png> [--size WxH]\ntantalus-client play --assets <StreamingAssets> --game-data <FF9_Data> --memoria <Memoria root> --atan <ratan_tbl.bin> [--battles <battle-scene-ids.tsv>] [--start 70] [--world-exits LOCATION,...] [--load SLOT|menu] [--save-dir DIR] [--load-save SavedData_ww.dat [--slot auto|N|SLOT:SAVE|list] [--save-to FILE]] [--ui-skin ps1|steam] [--psx-discs DIR] [--config FILE] [--scale N] [--mute] [--harness DIR [--script FROM-TO:HEXMASK,...|@FILE] [--auto-confirm N] [--frames N] [--shot-every N] [--harness-hz HZ] [--battle attack|row2|row4|mix|flee|idle] [--hurt 1]]\ntantalus-client capture-play <play flags> --ticks N [--hold FROM-TO:HEXMASK,...] [--auto-confirm N] [--party 0,1,2,3] [--hurt 1] [--give 2,3] [--presentation BUBBLE:HERE:FADEMODE:C:M:Y[:SCALE[:ATE]]] [--audio-log] [--audio-wav FILE] --output <png> [--size WxH]
tantalus-client capture-battle <play flags> --battles <tsv> [--ticks N] [--hold ...] [--auto-confirm N] [--battle N] [--at SECONDS,...] [--camera N] [--window-frame <png>] [--live 1] --output <png> [--size WxH]"
        );
        return ExitCode::SUCCESS;
    }
    match args.first().and_then(|arg| arg.to_str()) {
        Some("capture") => run_synthetic_capture(&args[1..]),
        Some("capture-field") => run_field_capture(&args[1..]),
        Some("view-field") => run_field_view(&args[1..]),
        Some("view-track") => run_track_view(&args[1..]),
        Some("capture-track") => run_track_capture(&args[1..]),
        Some("play") => run_play(&args[1..]),
        Some("capture-play") => run_play_capture(&args[1..]),
        Some("capture-battle") => battle_capture::run(&args[1..]),
        Some("capture-world") => run_world_capture(&args[1..]),
        Some("capture-card") => match tantalus_client::card_view::run_capture(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("capture-card failed: {error}");
                ExitCode::FAILURE
            }
        },
        Some("capture-movie") => match tantalus_client::movie_view::run_capture(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("capture-movie failed: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!(
                "expected `capture`, `capture-field` `view-field`, `view-track`, `capture-track`, `play` or `capture-play` (see --help)"
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
/// Removes a flag that takes no value from `args`; true when it was there.
fn take_switch(args: &[std::ffi::OsString], name: &str) -> (bool, Vec<std::ffi::OsString>) {
    let found = args.iter().any(|arg| arg == name);
    let rest = args.iter().filter(|arg| *arg != name).cloned().collect();
    (found, rest)
}

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
                    None,
                )
                .actors;
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

const PLAY_FLAGS: [&str; 17] = [
    "--ui-skin",
    "--psx-discs",
    "--world-map",
    "--assets",
    "--game-data",
    "--memoria",
    "--atan",
    "--battles",
    "--start",
    "--world-exits",
    "--language",
    "--load",
    "--save-dir",
    "--set-global",
    "--load-save",
    "--slot",
    "--save-to",
];

/// New game from the `play` flags, on its first field (events printed).
fn load_game(
    values: &std::collections::BTreeMap<String, String>,
) -> Result<tantalus_tools::play::PlayGame, String> {
    // `--ui-skin ps1|steam` and `--psx-discs DIR` (or the config's `ui_skin` / `psx_discs`).
    tantalus_client::ui_skin::set(tantalus_client::ui_skin::SkinSettings::resolve(
        values.get("--config").map(std::path::Path::new),
        values.get("--ui-skin").map(String::as_str),
        values.get("--psx-discs").map(String::as_str),
    )?);
    let path = |flag: &str| {
        values
            .get(flag)
            .map(PathBuf::from)
            .ok_or_else(|| format!("requires {flag}"))
    };
    let paths = tantalus_tools::play::PlayPaths {
        streaming_assets: path("--assets")?,
        game_data: path("--game-data")?,
        memoria: path("--memoria")?,
        atan_table: path("--atan")?,
        battles: values.get("--battles").map(PathBuf::from),
        language: values
            .get("--language")
            .cloned()
            .unwrap_or_else(|| "us".to_owned()),
    };
    let start = values
        .get("--start")
        .map_or(Ok(70), |value| value.parse().map_err(|_| "bad --start"))?;
    let data = tantalus_tools::play::PlayData::load(&paths)?;
    let globals = match values.get("--set-global") {
        // INDEX:KIND:VALUE,... (KIND bit, byte, i16, u16), a test aid that starts a probe at a
        // chosen story state: written before the field's scripts run.
        Some(list) => list
            .split(',')
            .map(|entry| {
                let parts: Vec<&str> = entry.split(':').collect();
                let parsed = (
                    parts.first().and_then(|v| v.parse::<u16>().ok()),
                    parts.get(1).and_then(|kind| match *kind {
                        "bit" => Some(tantalus_legacy::VariableType::Bit),
                        "byte" => Some(tantalus_legacy::VariableType::Byte),
                        "i16" => Some(tantalus_legacy::VariableType::Int16),
                        "u16" => Some(tantalus_legacy::VariableType::UInt16),
                        _ => None,
                    }),
                    parts.get(2).and_then(|v| v.parse::<i32>().ok()),
                );
                match parsed {
                    (Some(index), Some(kind), Some(value)) => Ok((index, kind, value)),
                    _ => Err(format!(
                        "bad --set-global entry `{entry}`, want INDEX:KIND:VALUE"
                    )),
                }
            })
            .collect::<Result<Vec<_>, String>>()?,
        None => Vec::new(),
    };
    let (mut game, events) =
        tantalus_tools::play::PlayGame::new_with_globals(data, start, &globals)?;
    for event in &events {
        println!("{}", tantalus_client::live::describe(event));
    }
    if let Some(dir) = values.get("--save-dir") {
        game.set_save_dir(PathBuf::from(dir));
    }
    match values.get("--load").map(String::as_str) {
        None => {}
        // A load screen over the first field.
        Some("menu") => game.open_load_screen(),
        Some(slot) => {
            let slot: usize = slot
                .parse()
                .map_err(|_| format!("--load wants a slot number or `menu`, got `{slot}`"))?;
            let mut events = Vec::new();
            game.load_slot(slot, &mut events)?;
            for event in &events {
                println!("{}", tantalus_client::live::describe(event));
            }
        }
    }
    if let Some(path) = values.get("--load-save") {
        // A Steam save file (`SavedData_ww.dat`, read-only). `--slot` is the file's own numbering:
        // `auto` (the autosave), a number 0 to 149 (`slot * 15 + save`), `SLOT:SAVE`, or `list` to
        // print the saves in the file. Without it the file's latest save. Saves from the menu go
        // to `--save-to`, by default `<file>.tantalus.dat` next to it.
        let path = PathBuf::from(path);
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let location = match values.get("--slot").map(String::as_str) {
            Some("list") => {
                for (location, line) in tantalus_tools::steam_save::list_saves(&bytes)? {
                    let name = if location.is_autosave {
                        "auto".to_owned()
                    } else {
                        format!(
                            "{} ({}:{})",
                            location.slot_id * 15 + location.save_id,
                            location.slot_id,
                            location.save_id
                        )
                    };
                    println!("slot {name}: {line}");
                }
                return Err("listed the saves; start one with --slot".to_owned());
            }
            Some(text) => tantalus_tools::steam_save::parse_location(text)?,
            None => tantalus_tools::steam_save::latest_location(&bytes)?,
        };
        let mut events = Vec::new();
        let resume = game.load_steam_save(&path, &location, &mut events)?;
        if let Some(output) = values.get("--save-to") {
            game.set_steam_output(PathBuf::from(output));
        }
        println!("loaded save: field {}", resume.map);
        if let Some((map, place)) = resume.world {
            println!("saved on the world map ({map}, {place}); entering its last field instead");
        }
        for event in &events {
            println!("{}", tantalus_client::live::describe(event));
        }
    }
    // The world map is always walked once a field leaves for it (`--world-map` is kept as a
    // no-op). `--world-exits` is a trace aid: its locations are used, in order, for the next
    // world exits instead of walking, and the world map runs again once they are used up.
    game.world_mode = true;
    if let Some(list) = values.get("--world-exits") {
        game.world_exits = list
            .split(',')
            .map(|value| {
                value
                    .trim()
                    .parse()
                    .map_err(|_| format!("bad world exit `{value}`"))
            })
            .collect::<Result<_, _>>()?;
    }
    Ok(game)
}

fn run_play(args: &[std::ffi::OsString]) -> ExitCode {
    // `--mute` leaves the window silent (no audio thread, no device).
    let (mute, args) = take_switch(args, "--mute");
    let args = &args[..];
    let mut allowed = PLAY_FLAGS.to_vec();
    allowed.extend([
        "--scale",
        "--fps-cap",
        "--fps-interp",
        "--config",
        "--harness",
        "--script",
        "--auto-confirm",
        "--frames",
        "--shot-every",
        "--harness-hz",
        "--battle",
        "--hurt",
    ]);
    let Some(values) = flag_values(args, &allowed) else {
        return ExitCode::FAILURE;
    };
    // `--fps-cap N` (0: no limit, default: the display's rate), `--fps-interp
    // on|off` and `--config FILE` (`fps_cap = N`, `interpolation = on|off`).
    let fps = match tantalus_client::smooth::FpsSettings::resolve(
        values.get("--config").map(std::path::Path::new),
        values.get("--fps-cap").map(String::as_str),
        values.get("--fps-interp").map(String::as_str),
    ) {
        Ok(fps) => fps,
        Err(error) => {
            eprintln!("play failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let scale = values
        .get("--scale")
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);
    let harness = match harness_settings(&values, &fps) {
        Ok(harness) => harness,
        Err(error) => {
            eprintln!("play failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = load_game(&values).and_then(|mut game| {
        // The PS1 skin shows the disc's own title menu when its art is there (at launch and
        // after a game over).
        game.set_ps1_title(tantalus_client::ui_skin::ps1_title_words().is_some());
        // `TitleUI` first, unless the launch line names a save or a start field.
        if !["--load", "--load-save", "--start"]
            .iter()
            .any(|flag| values.contains_key(*flag))
            && std::env::var_os("TANTALUS_BENCH_SCENE").is_none()
        {
            game.open_title(70);
        }
        // Frame-rate bench scenes (`TANTALUS_BENCH_SCENE=world:FIELD|battle:ID|cards:OPPONENT`).
        if let Ok(scene) = std::env::var("TANTALUS_BENCH_SCENE")
            && let Some((kind, number)) = scene.split_once(':')
            && let Ok(number) = number.parse::<i32>()
        {
            let mut events = Vec::new();
            match kind {
                "world" => game.start_world(Some(number), &mut events)?,
                "battle" => game.live_battles = true,
                "cards" => game.bench_card_game(number as u16),
                _ => return Err(format!("bad TANTALUS_BENCH_SCENE `{scene}`")),
            }
            if kind == "battle" {
                events = game.bench_battle(number);
            }
            for event in &events {
                println!("{}", tantalus_client::live::describe(event));
            }
        }
        // `--hurt 1`: the party at a quarter of its HP (a harness aid to reach a defeat).
        if values.contains_key("--hurt") {
            game.hurt_party();
        }
        let assets = PathBuf::from(&values["--assets"]);
        tantalus_client::live::play(game, assets, scale, mute, fps, harness)
            .map_err(|error| error.to_string())
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("play failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `play --harness DIR [--script FROM-TO:HEXMASK,...|@FILE] [--auto-confirm N] [--frames N]
/// [--shot-every N] [--harness-hz HZ]`: the play window hidden, driven by a button script on a
/// virtual clock, with a PNG every N frames and `frames.jsonl` in DIR (see `play_harness`).
fn harness_settings(
    values: &std::collections::BTreeMap<String, String>,
    fps: &tantalus_client::smooth::FpsSettings,
) -> Result<Option<tantalus_client::play_harness::HarnessSettings>, String> {
    let Some(out) = values.get("--harness") else {
        return Ok(None);
    };
    let number = |flag: &str, default: u64| -> Result<u64, String> {
        values.get(flag).map_or(Ok(default), |value| {
            value.parse().map_err(|_| format!("bad {flag}"))
        })
    };
    let hz = match values.get("--harness-hz") {
        Some(value) => value.parse().map_err(|_| "bad --harness-hz")?,
        None => match fps.cap {
            tantalus_client::smooth::FpsCap::Limit(rate) => f64::from(rate),
            _ => 60.0,
        },
    };
    Ok(Some(tantalus_client::play_harness::HarnessSettings {
        out: PathBuf::from(out),
        script: match values.get("--script") {
            Some(text) => tantalus_client::play_harness::parse_script(text)?,
            None => Vec::new(),
        },
        auto_confirm: number("--auto-confirm", 0)?,
        frames: number("--frames", 600)?,
        shot_every: number("--shot-every", 10)?,
        hz,
        battle: values
            .get("--battle")
            .map(|text| tantalus_client::play_harness::BattlePolicy::parse(text))
            .transpose()?,
    }))
}

/// `--hold FROM-TO:HEXMASK,...` ranges of held pad bits.
fn parse_holds(list: Option<&String>) -> Result<Vec<(u64, u64, i32)>, String> {
    let Some(list) = list else {
        return Ok(Vec::new());
    };
    list.split(',')
        .map(|part| {
            part.split_once(':')
                .and_then(|(range, mask)| {
                    let (from, to) = range.split_once('-')?;
                    Some((
                        from.parse().ok()?,
                        to.parse().ok()?,
                        i32::from_str_radix(mask.trim_start_matches("0x"), 16).ok()?,
                    ))
                })
                .ok_or_else(|| format!("bad --hold entry `{part}`, want FROM-TO:HEXMASK"))
        })
        .collect()
}

/// Headless check of the play window: runs the game with scripted input
/// (held ranges, plus a confirm press once a window has been open
/// `--auto-confirm` ticks), then renders the field it ends on with its
/// actors to a PNG.
fn run_play_capture(args: &[std::ffi::OsString]) -> ExitCode {
    // Battle and world frames draw on the GPU (`TANTALUS_RASTER_CPU=1`: the CPU).
    tantalus_client::frame_gpu::enable_headless();
    // `--audio-log` prints each sound request with its tick; no device is ever opened.
    let (audio_log, args) = take_switch(args, "--audio-log");
    let args = &args[..];
    let mut allowed = PLAY_FLAGS.to_vec();
    allowed.extend([
        "--ticks",
        "--hold",
        "--auto-confirm",
        "--place",
        "--output",
        "--size",
        "--party",
        "--hurt",
        "--give",
        "--learn",
        "--save-point",
        "--shop",
        "--chocograph",
        "--moogle-save",
        "--gil",
        "--audio-wav",
        "--presentation",
        "--bench-hz",
        "--bench-interp",
    ]);
    let Some(mut values) = flag_values(args, &allowed) else {
        return ExitCode::FAILURE;
    };
    if audio_log {
        values.insert("--audio-log".to_owned(), "1".to_owned());
    }
    match play_capture(&values) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("capture-play failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn play_capture(values: &std::collections::BTreeMap<String, String>) -> Result<String, String> {
    use tantalus_tools::play::pad;
    let output = values
        .get("--output")
        .map(PathBuf::from)
        .ok_or("requires --output")?;
    let ticks: u64 = values
        .get("--ticks")
        .map_or(Ok(600), |value| value.parse().map_err(|_| "bad --ticks"))?;
    let auto_confirm: u64 = values.get("--auto-confirm").map_or(Ok(0), |value| {
        value.parse().map_err(|_| "bad --auto-confirm")
    })?;
    let (width, height) = match values.get("--size") {
        None => (320, 240),
        Some(size) => parse_size(std::path::Path::new(size)).ok_or("--size must be WxH")?,
    };
    let holds = parse_holds(values.get("--hold"))?;
    tantalus_client::widescreen::set_target(width, height);
    // `--place TICK:X,Y,Z`: a test aid, moves the player there on that tick.
    let place: Option<(u64, [f32; 3])> = match values.get("--place") {
        Some(spec) => {
            let (tick, at) = spec.split_once(':').ok_or("--place wants TICK:X,Y,Z")?;
            let at: Vec<f32> = at.split(',').filter_map(|v| v.parse().ok()).collect();
            let at: [f32; 3] = at.try_into().map_err(|_| "--place wants TICK:X,Y,Z")?;
            Some((tick.parse().map_err(|_| "bad --place tick")?, at))
        }
        None => None,
    };
    let mut game = load_game(values)?;
    if let Some(party) = values.get("--party") {
        let ids: Vec<i32> = party.split(',').filter_map(|id| id.parse().ok()).collect();
        game.set_party(&ids);
    }
    if values.contains_key("--save-point") {
        game.set_save_point();
    }
    if let Some(gil) = values.get("--gil") {
        game.set_gil(gil.parse().map_err(|_| "bad --gil")?);
    }
    let shop: Option<i32> = match values.get("--shop") {
        Some(shop) => Some(shop.parse().map_err(|_| "bad --shop")?),
        None => None,
    };
    if let Some(list) = values.get("--learn") {
        for pair in list.split(',') {
            let parsed = pair
                .split_once(':')
                .and_then(|(who, id)| Some((who.parse().ok()?, id.parse().ok()?)));
            match parsed {
                Some((who, id)) if game.learn(who, id) => {}
                _ => {
                    return Err(format!(
                        "bad --learn entry `{pair}`, want CHARACTER:ABILITY"
                    ));
                }
            }
        }
    }
    if let Some(give) = values.get("--give") {
        let ids: Vec<i32> = give.split(',').filter_map(|id| id.parse().ok()).collect();
        game.give_items(&ids);
    }
    if let Some(spec) = values.get("--chocograph") {
        // FOUND_MASK[:REACH], a test aid: the world list with those pictures found.
        let (mask, reach) = spec.split_once(':').unwrap_or((spec.as_str(), "2"));
        let mask = parse_number::<u32>(mask).ok_or("bad --chocograph mask")?;
        let reach = reach.parse().map_err(|_| "bad --chocograph reach")?;
        let mut events = Vec::new();
        game.open_chocograph(mask, reach, &mut events)?;
    }
    if values.contains_key("--hurt") {
        game.hurt_party();
    }
    let mut audio = CaptureAudio::new(values, &game)?;
    let mut open_for = 0u64;
    let mut last_confirm = false;
    let mut view = tantalus_client::view_scroll::ViewScroll::default();
    // Layer scrolling, started once a field's scripts call a layer opcode.
    let mut motion: Option<(
        String,
        tantalus_client::overlay_motion::OverlayMotionState,
        tantalus_contracts::FieldSceneDescription,
    )> = None;
    // The tick each open window first showed, so the capture's text has typed in as it
    // would have on screen (see `dialog_view::seed_opened`).
    let mut dialog_opened: std::collections::HashMap<(u8, u16), u64> = Default::default();
    for tick in 0..ticks {
        let mut held = holds
            .iter()
            .filter(|(from, to, _)| (*from..*to).contains(&tick))
            .fold(0, |mask, (_, _, bits)| mask | bits);
        open_for = if game.open_text().is_empty() {
            0
        } else {
            open_for + 1
        };
        // Release between presses so each one is a new press.
        if auto_confirm > 0 && open_for >= auto_confirm && !last_confirm {
            held |= pad::CONFIRM;
            open_for = 0;
        }
        last_confirm = held & pad::CONFIRM != 0;
        if let Some((at_tick, at)) = place
            && tick == at_tick
        {
            game.place_player(at);
        }
        // The scripts of a shop or a moogle run once the field is up.
        if tick == 10 {
            if let Some(shop) = shop {
                game.open_shop(shop);
            }
            if values.contains_key("--moogle-save") {
                game.request_save();
            }
        }
        for event in game.tick(held)? {
            println!("{}", tantalus_client::live::describe(&event));
            if let tantalus_tools::play::PlayEvent::Dialog { choice: true, .. } = event {
                // The tick to pass as `--ticks` for a capture with the choice cursor up.
                println!("   (choice opened at tick {tick})");
            }
        }
        if std::env::var_os("TANTALUS_SPS_LOG").is_some() {
            let slots: Vec<String> = game
                .world()
                .sps
                .slots
                .iter()
                .enumerate()
                .filter(|(_, slot)| slot.loaded)
                .map(|(index, slot)| {
                    format!(
                        "{index}:{}@{} attr {}",
                        slot.id,
                        slot.cur_frame >> 4,
                        slot.attr
                    )
                })
                .collect();
            if tick % 10 == 0 || !slots.is_empty() && tick % 3 == 0 {
                println!("tick {tick}: sps [{}]", slots.join(", "));
            }
        }
        audio.tick(&mut game)?;
        let now_open: Vec<(u8, u16)> = game
            .open_dialogs()
            .iter()
            .map(|d| (d.window, d.text))
            .collect();
        dialog_opened.retain(|key, _| now_open.contains(key));
        for key in now_open {
            dialog_opened.entry(key).or_insert(game.ticks);
        }
        tantalus_client::live::observe_view(&mut view, &game);
        let background = game.background().map(str::to_owned);
        if motion.as_ref().map(|(name, ..)| name) != background.as_ref() {
            motion = None;
        }
        if motion.is_none()
            && !game.world().overlay_motion.is_empty()
            && let Some(name) = &background
            && let Ok(mut field) =
                tantalus_client::window::find_field(std::path::Path::new(&values["--assets"]), name)
        {
            if let Some(scene) = field
                .scene
                .with_camera(u32::from(game.world().field_camera))
            {
                field.scene = scene;
            }
            motion = tantalus_client::overlay_motion::OverlayMotionState::new(
                &field.background,
                &field.scene,
            )
            .map(|state| (name.clone(), state, field.scene.clone()));
        }
        if let Some((_, state, scene)) = &mut motion {
            let at = if state.needs_camera() {
                // The scroll range reads the widescreen view of this field.
                tantalus_client::live::note_field_view(&game);
                let (frame, _) = tantalus_client::live::world_frame(game.world(), game.ticks);
                view.view(
                    &game.world().camera_log,
                    &scene.camera,
                    scene.camera_index,
                    tantalus_client::live::player_position(&frame),
                    game.ticks,
                )
            } else {
                None
            };
            state.map = game.map().unwrap_or(-1);
            state.tick(&game.world().overlay_motion, at);
        }
        if game.stopped.is_some() {
            break;
        }
    }
    audio.finish()?;
    if let Some(spec) = values.get("--presentation") {
        // BUBBLE:HERE:FADEMODE:C:M:Y[:SCALE[:ATE]], a test aid; a negative bubble or fade mode is off.
        let numbers: Vec<i32> = spec
            .split(':')
            .filter_map(|part| part.parse().ok())
            .collect();
        if let &[bubble, here, mode, c, m, y, ref rest @ ..] = numbers.as_slice() {
            game.show_presentation(
                (bubble >= 0).then_some(bubble),
                here,
                (mode >= 0).then_some((mode, [c, m, y])),
                rest.first().copied(),
                rest.get(1).copied(),
            );
        } else {
            return Err(
                "bad --presentation, want BUBBLE:HERE:FADEMODE:C:M:Y[:SCALE[:ATE]]".to_owned(),
            );
        }
    }
    if let Some(change) = game.disc_change() {
        // Between discs: black, the disc picture over it while it shows, then the save screen.
        let mut base = image::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
        if let Some((path, alpha)) = game.disc_picture() {
            let picture = image::open(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?
                .to_rgba8();
            let size = tantalus_client::disc_view::picture_size(picture.width(), picture.height());
            let scale = height as f32 / 240.0;
            let (w, h) = ((size.x * scale) as u32, (size.y * scale) as u32);
            let scaled =
                image::imageops::resize(&picture, w, h, image::imageops::FilterType::Triangle);
            let left = (i64::from(width) - i64::from(w)) / 2;
            for (x, y, pixel) in scaled.enumerate_pixels() {
                let (bx, by) = (left + i64::from(x), i64::from(y));
                if bx < 0 || bx >= i64::from(width) || by >= i64::from(height) {
                    continue;
                }
                let a = alpha * f32::from(pixel[3]) / 255.0;
                let target = base.get_pixel_mut(bx as u32, by as u32);
                for (to, from) in target.0.iter_mut().zip(pixel.0).take(3) {
                    *to = (f32::from(from) * a + f32::from(*to) * (1.0 - a)) as u8;
                }
            }
        }
        base.save(&output)
            .map_err(|error| format!("cannot write {}: {error}", output.display()))?;
        overlay_menu(&game, &output)?;
        return Ok(format!(
            "saved the change to disc {} ({:?}, field {} next) after {} ticks, to {}",
            change.disc + 1,
            change.stage,
            change.field,
            game.ticks,
            output.display()
        ));
    }
    if let Some(play) = game.world_play.as_ref() {
        let mut renderer = tantalus_client::world_view::WorldRenderer::new(std::path::Path::new(
            &values["--assets"],
        ));
        let frame = renderer.render(play, width, height);
        tantalus_client::battle_view::save_png(&frame, &output)?;
        overlay_menu(&game, &output)?;
        return Ok(format!(
            "saved the world map at ({:.1}, {:.1}) after {} ticks, to {}",
            play.walker.pos.x,
            play.walker.pos.z,
            game.ticks,
            output.display()
        ));
    }
    let background = game
        .background()
        .ok_or("the run ended on no field background")?;
    let mut field =
        tantalus_client::window::find_field(std::path::Path::new(&values["--assets"]), background)
            .map_err(|error| error.to_string())?;
    if let Some(scene) = field
        .scene
        .with_camera(u32::from(game.world().field_camera))
    {
        field.scene = scene;
    }
    let (frame, placed) = tantalus_client::live::world_frame(game.world(), game.ticks);
    let mut models =
        tantalus_client::actor_models::ActorModels::new(std::path::Path::new(&values["--assets"]));
    for problem in models.prepare_field(game.field_name()) {
        println!("!! field light: {problem}");
    }
    if std::env::var_os("TANTALUS_PERF").is_some() {
        println!(
            "perf light scripts: {:?} {:?}",
            game.world().actor_shadows,
            game.world().light_changes
        );
    }
    models.apply_light_changes(&game.world().light_changes);
    for problem in models.prepare(frame.poses.values()) {
        println!("!! actor models: {problem}");
    }
    models.advance_actor_tex_anims(
        game.ticks,
        &frame.poses,
        &game.world().tex_anims,
        game.world().tex_anim_clock,
    );
    if std::env::var_os("TANTALUS_PERF").is_some() {
        play_perf(&game, &models, &field.scene.camera);
    }
    let sps_library = tantalus_client::sps_view::library(std::path::Path::new(&values["--assets"]));
    let mut drawn = tantalus_client::track::Drawn::default();
    let mut sps_drawn = 0;
    tantalus_client::capture::capture_field_scene(
        &field,
        &output,
        CaptureConfig {
            width,
            height,
            ..CaptureConfig::default()
        },
        |world| {
            let started = std::time::Instant::now();
            drawn = tantalus_client::track::spawn_frame_markers(
                world,
                &field.scene.camera,
                &frame,
                &placed,
                Some(&models),
            );
            if std::env::var_os("TANTALUS_PERF").is_some() {
                println!(
                    "perf spawn_frame_markers (rasters on threads + spawn): {:.3} ms",
                    started.elapsed().as_secs_f64() * 1000.0
                );
                // Mean of 20 more runs (each replaces the markers).
                let again = std::time::Instant::now();
                for _ in 0..20 {
                    tantalus_client::track::spawn_frame_markers(
                        world,
                        &field.scene.camera,
                        &frame,
                        &placed,
                        Some(&models),
                    );
                }
                println!(
                    "perf spawn_frame_markers mean of 20: {:.3} ms",
                    again.elapsed().as_secs_f64() * 1000.0 / 20.0
                );
            }
            sps_drawn = tantalus_client::sps_view::sync_game_sps(
                world,
                &game,
                &field.scene.camera,
                &frame,
                Some(&models),
                &sps_library,
                &game.world().sps.slots,
            );
            tantalus_client::scene_adapter::apply_overlay_changes(
                world,
                &field.scene,
                &game.world().overlays,
                tantalus_client::live::player_position(&frame),
            );
            tantalus_client::scene_adapter::apply_tile_shade(world, &game.world().tile_shade);
            tantalus_client::live::note_field_view(&game);
            let at = view.view(
                &game.world().camera_log,
                &field.scene.camera,
                field.scene.camera_index,
                tantalus_client::live::player_position(&frame),
                game.ticks,
            );
            tantalus_client::live::place_camera(world, at);
            tantalus_client::window::fit_field_frame(world);
            if let Some((_, state, _)) = &mut motion {
                state.place_sprites(world, &field.scene, 1.0);
            }
            // `TANTALUS_BENCH_FRAMES=N --bench-hz H`: N display frames at H Hz
            // of the field's loop (logic at 30 Hz, the markers drawn between
            // ticks), main-thread milliseconds, no window.
            if let Some(frames) = tantalus_client::live::bench::frames() {
                let hz = bench_hz(values);
                field_bench(
                    world,
                    &mut game,
                    &mut models,
                    &field.scene.camera,
                    frames,
                    hz,
                    bench_interp(values, hz),
                );
            }
            tantalus_client::dialog_view::seed_opened(world, &dialog_opened);
            tantalus_client::dialog_view::sync_dialogs(world, &game, &field.scene.camera, &frame);
            tantalus_client::presentation_view::sync_presentation(
                world,
                game.world(),
                game.game_data(),
                &field.scene.camera,
                &frame,
                game.ticks,
            );
            tantalus_client::menu_view::sync_menu(world, &game);
        },
    )
    .map_err(|error| error.to_string())?;
    let player = game
        .world()
        .player()
        .and_then(|player| game.world().actors.get(&player))
        .map(|actor| actor.position);
    println!(
        "sps drawn: {} of {} loaded slots",
        sps_drawn,
        game.world()
            .sps
            .slots
            .iter()
            .filter(|slot| slot.loaded)
            .count()
    );
    // `TANTALUS_SPS_LOG=1`: one line per loaded effect.
    if std::env::var_os("TANTALUS_SPS_LOG").is_some() {
        for (index, slot) in game.world().sps.slots.iter().enumerate() {
            if slot.loaded {
                println!("sps slot {index}: {slot:?}");
            }
        }
    }
    Ok(format!(
        "saved field {} ({}) after {} ticks with {} actors ({} as models), player at {player:?}, bubble {:?} here {} fade {:?}, {} attached, to {}",
        game.map().unwrap_or(-1),
        game.field_name().unwrap_or("-"),
        game.ticks,
        drawn.actors,
        drawn.models,
        game.world().bubble_icon,
        game.world().here_icon,
        game.world().fade_filter,
        game.world()
            .attachments
            .values()
            .filter(|held| held.carrier.is_some())
            .count(),
        output.display()
    ))
}

/// Per-frame cost of the live window's pieces, printed with `TANTALUS_PERF=1`
/// on `capture-play`: 30 runs of each, mean milliseconds.
fn play_perf(
    game: &tantalus_tools::play::PlayGame,
    models: &tantalus_client::actor_models::ActorModels,
    camera: &tantalus_contracts::LegacyFieldCamera,
) {
    use std::time::Instant;
    const RUNS: u32 = 30;
    let mean = |label: &str, mut run: Box<dyn FnMut() + '_>| {
        run();
        let start = Instant::now();
        for _ in 0..RUNS {
            run();
        }
        println!(
            "perf {label}: {:.3} ms",
            start.elapsed().as_secs_f64() * 1000.0 / f64::from(RUNS)
        );
    };
    mean(
        "world_frame",
        Box::new(|| {
            std::hint::black_box(tantalus_client::live::world_frame(game.world(), game.ticks));
        }),
    );
    let (frame, _) = tantalus_client::live::world_frame(game.world(), game.ticks);
    mean(
        "render all actors",
        Box::new(|| {
            for (id, position) in &frame.actors {
                if let Some(pose) = frame.poses.get(id) {
                    std::hint::black_box(models.render(camera, *position, pose));
                }
            }
        }),
    );
    mean(
        "render_shadow all actors",
        Box::new(|| {
            for (id, position) in &frame.actors {
                if let Some(pose) = frame.poses.get(id) {
                    std::hint::black_box(models.render_shadow(camera, *position, pose));
                }
            }
        }),
    );
    mean(
        "shadow_quad all actors",
        Box::new(|| {
            for (id, position) in &frame.actors {
                if let Some(pose) = frame.poses.get(id) {
                    std::hint::black_box(models.shadow_quad(camera, *position, pose));
                }
            }
        }),
    );
    let drawn = frame
        .actors
        .iter()
        .filter(|(id, p)| {
            frame
                .poses
                .get(id)
                .is_some_and(|pose| pose.visible && models.render(camera, *p, pose).is_some())
        })
        .count();
    println!(
        "perf actors with a raster: {drawn} of {}",
        frame.actors.len()
    );
}

/// Sound in `capture-play`: off unless `--audio-log` (print each request with its tick) or
/// `--audio-wav FILE` (also render the run's sound to a WAV) is given. Neither opens a device.
struct CaptureAudio {
    log: Option<tantalus_tools::sound_bank::SoundBank>,
    wav: Option<(
        tantalus_tools::play_audio::AudioDirector,
        tantalus_assets::pc::audio_decode::WavWriter<std::io::BufWriter<std::fs::File>>,
    )>,
    /// The song the WAV director plays, printed with `--audio-log` when it changes.
    music: i32,
}

/// The sample rate of the WAV `--audio-wav` writes (a multiple of 60).
const WAV_RATE: u32 = 44_100;

impl CaptureAudio {
    fn new(
        values: &std::collections::BTreeMap<String, String>,
        game: &tantalus_tools::play::PlayGame,
    ) -> Result<Self, String> {
        use tantalus_tools::sound_bank::SoundBank;
        let open = || SoundBank::open(std::path::Path::new(&values["--assets"]), game.game_data());
        let log = values.contains_key("--audio-log").then(open).transpose()?;
        let wav = match values.get("--audio-wav") {
            Some(path) => {
                let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
                let writer = tantalus_assets::pc::audio_decode::WavWriter::new(
                    std::io::BufWriter::new(file),
                    2,
                    WAV_RATE,
                )
                .map_err(|e| e.to_string())?;
                let director = tantalus_tools::play_audio::AudioDirector::new(open()?, WAV_RATE)?;
                Some((director, writer))
            }
            None => None,
        };
        Ok(Self {
            log,
            wav,
            music: -1,
        })
    }

    /// Handles the sound requests of the tick `game` just played. A game tick is two 1/60 s
    /// sound ticks.
    fn tick(&mut self, game: &mut tantalus_tools::play::PlayGame) -> Result<(), String> {
        if self.log.is_none() && self.wav.is_none() {
            return Ok(());
        }
        let cues = game.take_audio();
        if let Some(bank) = &self.log {
            for (tick, cue) in &cues {
                println!(
                    "audio tick {tick}: {}",
                    tantalus_tools::play_audio::describe_cue(cue, bank)
                );
            }
        }
        if let Some((director, writer)) = &mut self.wav {
            for (_, cue) in &cues {
                director.apply(cue);
            }
            let samples = director.render(2);
            let music = director.system.current_music_id();
            if self.log.is_some() && music != self.music {
                println!("audio tick {}: now playing music {music}", game.ticks);
                self.music = music;
            }
            let pcm: Vec<i16> = samples
                .iter()
                .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect();
            writer.write_samples(&pcm).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        if let Some((_, writer)) = self.wav.take() {
            writer.finish().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// `capture-world <play flags> [--world-at X,Z|script] --ticks N [--hold ...]
/// [--shots T,T] --output <png> [--size WxH]`: walks the world map
/// headlessly with scripted input and renders what the world camera sees.
fn run_world_capture(args: &[std::ffi::OsString]) -> ExitCode {
    // Battle and world frames draw on the GPU (`TANTALUS_RASTER_CPU=1`: the CPU).
    tantalus_client::frame_gpu::enable_headless();
    let mut allowed = PLAY_FLAGS.to_vec();
    allowed.extend([
        "--ticks",
        "--hold",
        "--output",
        "--size",
        "--world-at",
        "--arrive-from",
        "--shots",
        "--scenario",
        "--discovered",
        "--visited",
        "--ride",
        "--chocobo",
        "--world-script",
        "--bench-hz",
        "--bench-interp",
    ]);
    let Some(values) = flag_values(args, &allowed) else {
        return ExitCode::FAILURE;
    };
    match world_capture(&values) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("capture-world failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn world_capture(values: &std::collections::BTreeMap<String, String>) -> Result<String, String> {
    let output = values
        .get("--output")
        .map(PathBuf::from)
        .ok_or("requires --output")?;
    let ticks: u64 = values
        .get("--ticks")
        .map_or(Ok(0), |value| value.parse().map_err(|_| "bad --ticks"))?;
    let (width, height) = match values.get("--size") {
        None => (640, 480),
        Some(size) => parse_size(std::path::Path::new(size)).ok_or("--size must be WxH")?,
    };
    tantalus_client::widescreen::set_target(width, height);
    let holds = parse_holds(values.get("--hold"))?;
    let shots: Vec<u64> = values
        .get("--shots")
        .map(|list| list.split(',').filter_map(|t| t.parse().ok()).collect())
        .unwrap_or_default();
    // `--world-at script`: where the world script puts the character.
    let from_script = values.get("--world-at").is_some_and(|at| at == "script");
    let (x, z) = match values.get("--world-at").filter(|at| *at != "script") {
        Some(at) => at
            .split_once(',')
            .and_then(|(x, z)| Some((x.parse::<f32>().ok()?, z.parse::<f32>().ok()?)))
            .ok_or("--world-at wants X,Z")?,
        None => (1218.0, -747.0),
    };
    let mut game = load_game(values)?;
    if values.contains_key("--scenario")
        || values.contains_key("--visited")
        || values.contains_key("--discovered")
    {
        let number = |flag: &str| -> Result<i32, String> {
            values.get(flag).map_or(Ok(0), |value| {
                value.parse().map_err(|_| format!("bad {flag}"))
            })
        };
        let visited = values.get("--visited").map_or(Ok(0), |value| {
            u64::from_str_radix(value.trim_start_matches("0x"), 16).map_err(|_| "bad --visited")
        })?;
        game.set_story(number("--scenario")?, number("--discovered")?, visited);
    }
    if let Some(map) = values.get("--world-script") {
        // 9000 to 9009: which world's script runs (its ships and objects).
        game.world_script_map = map.parse().map_err(|_| "bad --world-script")?;
    }
    if let Some(kind) = values.get("--chocobo") {
        game.set_chocobo_kind(kind.parse().map_err(|_| "bad --chocobo")?);
    }
    if let Some(from) = values.get("--arrive-from") {
        // Arrive as if leaving field `from` for the world map.
        let from: i32 = from.parse().map_err(|_| "bad --arrive-from")?;
        let mut events = Vec::new();
        game.start_world(Some(from), &mut events)?;
        for event in &events {
            println!("{}", tantalus_client::live::describe(event));
        }
    } else if from_script {
        game.resume_world()?;
    } else {
        game.begin_world(x, z)?;
    }
    if let Some(ride) = values.get("--ride")
        && let Some(play) = game.world_play.as_mut()
    {
        play.set_ride(ride.parse().map_err(|_| "bad --ride")?);
    }
    if let Some(play) = game.world_play.as_ref() {
        for parked in play.parked_ships() {
            println!("parked {:?} at {:?}", parked.ship, parked.pos);
        }
    }
    let mut renderer =
        tantalus_client::world_view::WorldRenderer::new(std::path::Path::new(&values["--assets"]));
    let render = |renderer: &mut tantalus_client::world_view::WorldRenderer,
                  game: &tantalus_tools::play::PlayGame,
                  path: &std::path::Path| {
        let Some(play) = game.world_play.as_ref() else {
            return Ok(());
        };
        let frame = renderer.render(play, width, height);
        tantalus_client::battle_view::save_png(&frame, path)
    };
    for tick in 0..ticks {
        let held = holds
            .iter()
            .filter(|(from, to, _)| (*from..*to).contains(&tick))
            .fold(0, |mask, (_, _, bits)| mask | bits);
        for event in game.tick(held)? {
            println!("{}", tantalus_client::live::describe(&event));
        }
        if shots.contains(&(tick + 1)) {
            let stem = output
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("world");
            render(
                &mut renderer,
                &game,
                &output.with_file_name(format!("{stem}-{}.png", tick + 1)),
            )?;
        }
    }
    if let Some(frames) = tantalus_client::live::bench::frames() {
        use tantalus_client::{live::bench, smooth};
        let hz = bench_hz(values);
        let interpolate = bench_interp(values, hz);
        let mut pair = smooth::Pair::default();
        let Some(play) = game.world_play.as_ref() else {
            return Err("not on the world map".to_owned());
        };
        pair.push(tantalus_client::world_view::pose_of(play), true);
        let mut elapsed = 0.0;
        for frame_number in 0..frames + BENCH_WARMUP {
            if frame_number == BENCH_WARMUP {
                bench::reset();
            }
            let start = std::time::Instant::now();
            elapsed += 1.0 / hz;
            while elapsed >= smooth::LOGIC_STEP {
                elapsed -= smooth::LOGIC_STEP;
                game.tick(0)?;
                if let Some(play) = game.world_play.as_ref() {
                    pair.push(tantalus_client::world_view::pose_of(play), false);
                }
            }
            let Some(play) = game.world_play.as_ref() else {
                break;
            };
            let pose = pair.world(smooth::alpha(elapsed, interpolate));
            std::hint::black_box(renderer.render_blended(
                play,
                tantalus_client::world_view::LIVE_SIZE.0,
                tantalus_client::world_view::LIVE_SIZE.1,
                pose.as_ref(),
            ));
            bench::record("frame_cpu", start.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "bench world {hz} Hz, interpolation {}, logic 30 Hz",
            if interpolate { "on" } else { "off" }
        );
        bench::report();
    }
    let Some(play) = game.world_play.as_ref() else {
        return Ok(format!(
            "left the world map for field {:?} ({})",
            game.map(),
            game.field_name().unwrap_or("?")
        ));
    };
    render(&mut renderer, &game, &output)?;
    let pos = play.walker.pos;
    Ok(format!(
        "forms {:?}; world at ({:.1}, {:.1}, {:.1}) yaw {:.1} camera {:.1} block {:?} ground id {} place {:?} danger steps {}; {} problems; wrote {}",
        play.forms,
        pos.x,
        pos.y,
        pos.z,
        play.walker.rot_y,
        play.walker.camera.rotation,
        tantalus_legacy::world::block_index(pos.x, pos.z),
        play.walker.status.id,
        play.place,
        play.steps,
        renderer.problems.len(),
        output.display()
    ))
}

/// Frames a headless bench runs, untimed, before it measures.
const BENCH_WARMUP: usize = 60;

/// `--bench-hz H`: the simulated display rate of a headless frame bench.
fn bench_hz(values: &std::collections::BTreeMap<String, String>) -> f64 {
    values
        .get("--bench-hz")
        .and_then(|value| value.parse().ok())
        .unwrap_or(60.0)
}

/// `--bench-interp on|off`: blend the drawn state (default on above 30 Hz).
fn bench_interp(values: &std::collections::BTreeMap<String, String>, hz: f64) -> bool {
    match values.get("--bench-interp").map(String::as_str) {
        Some("off") => false,
        Some(_) => true,
        None => hz > 30.0,
    }
}

/// The field's display loop without a window: `frames` frames of `1 / hz`
/// seconds, the game ticking at its 30 Hz, the actors drawn between the last
/// two logic frames as `live.rs` does. Prints per-frame main-thread time.
fn field_bench(
    world: &mut bevy::ecs::world::World,
    game: &mut tantalus_tools::play::PlayGame,
    models: &mut tantalus_client::actor_models::ActorModels,
    camera: &tantalus_contracts::LegacyFieldCamera,
    frames: usize,
    hz: f64,
    interpolate: bool,
) {
    use tantalus_client::{live::bench, smooth};
    let (first, mut placed) = tantalus_client::live::world_frame(game.world(), game.ticks);
    let mut pair = smooth::Pair::default();
    pair.push(first, true);
    let mut elapsed = 0.0;
    let mut drawn_alpha = -1.0f32;
    let mut new_tick = true;
    // The first frames load models and clips; they are not timed.
    for frame_number in 0..frames + BENCH_WARMUP {
        if frame_number == BENCH_WARMUP {
            bench::reset();
        }
        let start = std::time::Instant::now();
        elapsed += 1.0 / hz;
        while elapsed >= smooth::LOGIC_STEP {
            elapsed -= smooth::LOGIC_STEP;
            let tick_start = std::time::Instant::now();
            if game.tick(0).is_err() {
                break;
            }
            bench::record("logic_tick", tick_start.elapsed().as_secs_f64() * 1000.0);
            let (frame, now_placed) = tantalus_client::live::world_frame(game.world(), game.ticks);
            models.prepare(frame.poses.values());
            models.advance_actor_tex_anims(
                game.ticks,
                &frame.poses,
                &game.world().tex_anims,
                game.world().tex_anim_clock,
            );
            pair.push(frame, false);
            placed = now_placed;
            new_tick = true;
        }
        let t = smooth::alpha(elapsed, interpolate);
        if new_tick || (interpolate && t != drawn_alpha) {
            let draw_start = std::time::Instant::now();
            let blended = pair.frame(t).expect("a frame was pushed");
            tantalus_client::track::spawn_frame_markers(
                world,
                camera,
                &blended,
                &placed,
                Some(models),
            );
            drawn_alpha = t;
            new_tick = false;
            bench::record("draw", draw_start.elapsed().as_secs_f64() * 1000.0);
        }
        bench::record("frame_cpu", start.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "bench field {hz} Hz, interpolation {}, logic 30 Hz",
        if interpolate { "on" } else { "off" }
    );
    bench::report();
}

/// A decimal or `0x` hexadecimal number.
fn parse_number<T: TryFrom<u64>>(text: &str) -> Option<T> {
    let value = match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok()?,
        None => text.parse().ok()?,
    };
    T::try_from(value).ok()
}

/// Draws the open menu (the Chocograph list) over a saved world capture,
/// each window as `DialogAssets::render_window` pictures laid out in PSX
/// pixels on the 320x240 frame, as the window's menu view does.
fn overlay_menu(
    game: &tantalus_tools::play::PlayGame,
    output: &std::path::Path,
) -> Result<(), String> {
    use tantalus_assets::pc::dialog_render::{
        BORDER_PADDING, DEFAULT_TEXT_COLOR, RESOURCE_X_MULTIPLIER, RESOURCE_Y_MULTIPLIER,
        WindowText,
    };
    use tantalus_tools::field_menu::Tint;
    let Some((_, frame)) = game.menu_frame() else {
        return Ok(());
    };
    let assets = tantalus_client::ui_skin::dialog_assets(game.game_data())
        .map_err(|error| format!("menu art: {error}"))?;
    let mut base = image::open(output)
        .map_err(|error| format!("cannot read {}: {error}", output.display()))?
        .to_rgba8();
    // The menu keeps its 4:3 area, centered in a wider picture.
    let sy = base.height() as f32 / 240.0;
    let sx = sy;
    let left = (base.width() as f32 - 320.0 * sx) / 2.0;
    let border = BORDER_PADDING as f32 / RESOURCE_X_MULTIPLIER;
    let color = |tint: Tint| {
        let [r, g, b, a] = DEFAULT_TEXT_COLOR;
        match tint {
            Tint::White => [r, g, b, a],
            Tint::Dim => [0.45, 0.45, 0.5, 1.0],
            Tint::Yellow => [0.96, 0.82, 0.4, 1.0],
            Tint::Green => [0.45, 0.95, 0.5, 1.0],
            Tint::Red => [1.0, 0.45, 0.4, 1.0],
            Tint::Cyan => [0.45, 0.9, 0.95, 1.0],
            Tint::Magenta => [0.95, 0.5, 0.9, 1.0],
        }
    };
    for window in &frame.windows {
        let texts: Vec<WindowText> = window
            .texts
            .iter()
            .map(|run| WindowText {
                x: run.x * RESOURCE_X_MULTIPLIER,
                y: run.y * RESOURCE_Y_MULTIPLIER,
                text: run.text.clone(),
                color: color(run.tint),
                right: run.right,
            })
            .collect();
        let picture = assets.render_window(
            (window.w * RESOURCE_X_MULTIPLIER).round() as i32,
            (window.h * RESOURCE_Y_MULTIPLIER).round() as i32,
            &texts,
        );
        let Some(source) = image::RgbaImage::from_raw(
            picture.width as u32,
            picture.height as u32,
            picture.rgba.clone(),
        ) else {
            continue;
        };
        let width = (picture.width as f32 / RESOURCE_X_MULTIPLIER * sx)
            .round()
            .max(1.0) as u32;
        let height = (picture.height as f32 / RESOURCE_Y_MULTIPLIER * 224.0 / 240.0 * sy)
            .round()
            .max(1.0) as u32;
        let scaled = image::imageops::resize(
            &source,
            width,
            height,
            image::imageops::FilterType::Triangle,
        );
        let x = (left + (window.x - border) * sx).round() as i64;
        let y = (((window.y - border) + 8.0) * sy).round() as i64;
        image::imageops::overlay(&mut base, &scaled, x, y);
    }
    if let Some((px, py)) = frame.pointer {
        let (cx, cy) = ((left + px * sx) as i64, ((py + 8.0) * sy) as i64);
        for (dx, dy) in (-5i64..=5).flat_map(|dx| (-5i64..=5).map(move |dy| (dx, dy))) {
            if dx.abs() + dy.abs() <= 5
                && let (Ok(x), Ok(y)) = (u32::try_from(cx + dx), u32::try_from(cy + dy))
                && x < base.width()
                && y < base.height()
            {
                base.put_pixel(x, y, image::Rgba([255, 242, 153, 255]));
            }
        }
    }
    base.save(output)
        .map_err(|error| format!("cannot write {}: {error}", output.display()))
}
