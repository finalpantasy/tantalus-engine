mod battle_capture;

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
            "tantalus-client capture --mode <2d|3d|hybrid> --output <png>\ntantalus-client capture-field --bundle <unityraw> --field <field> --output <png> [--size WxH]\ntantalus-client view-field --bundle <unityraw> --field <name> [--scale N]\ntantalus-client view-track --assets <StreamingAssets> --track <NNN_MAP.jsonl> [--scale N]\ntantalus-client capture-track --assets <StreamingAssets> --track <file> --tick T --output <png> [--size WxH]\ntantalus-client play --assets <StreamingAssets> --game-data <FF9_Data> --memoria <Memoria root> --atan <ratan_tbl.bin> [--battles <battle-scene-ids.tsv>] [--start 70] [--world-exits LOCATION,...] [--load SLOT|menu] [--save-dir DIR] [--scale N] [--mute]\ntantalus-client capture-play <play flags> --ticks N [--hold FROM-TO:HEXMASK,...] [--auto-confirm N] [--party 0,1,2,3] [--hurt 1] [--give 2,3] [--audio-log] [--audio-wav FILE] --output <png> [--size WxH]
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

const PLAY_FLAGS: [&str; 11] = [
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
];

/// New game from the `play` flags, on its first field (events printed).
fn load_game(
    values: &std::collections::BTreeMap<String, String>,
) -> Result<tantalus_tools::play::PlayGame, String> {
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
    let (mut game, events) = tantalus_tools::play::PlayGame::new(data, start)?;
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
    game.world_mode = values.contains_key("--world-map");
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
    allowed.push("--scale");
    let Some(values) = flag_values(args, &allowed) else {
        return ExitCode::FAILURE;
    };
    let scale = values
        .get("--scale")
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);
    let result = load_game(&values).and_then(|mut game| {
        // Without scripted exits the world map is walked.
        game.world_mode = !values.contains_key("--world-exits");
        let assets = PathBuf::from(&values["--assets"]);
        tantalus_client::live::play(game, assets, scale, mute).map_err(|error| error.to_string())
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("play failed: {error}");
            ExitCode::FAILURE
        }
    }
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
    // `--audio-log` prints each sound request with its tick; no device is ever opened.
    let (audio_log, args) = take_switch(args, "--audio-log");
    let args = &args[..];
    let mut allowed = PLAY_FLAGS.to_vec();
    allowed.extend([
        "--ticks",
        "--hold",
        "--auto-confirm",
        "--output",
        "--size",
        "--party",
        "--hurt",
        "--give",
        "--learn",
        "--save-point",
        "--audio-wav",
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
    let mut game = load_game(values)?;
    if let Some(party) = values.get("--party") {
        let ids: Vec<i32> = party.split(',').filter_map(|id| id.parse().ok()).collect();
        game.set_party(&ids);
    }
    if values.contains_key("--save-point") {
        game.set_save_point();
    }
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
        for event in game.tick(held)? {
            println!("{}", tantalus_client::live::describe(&event));
        }
        audio.tick(&mut game)?;
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
            state.tick(&game.world().overlay_motion, at);
        }
        if game.stopped.is_some() {
            break;
        }
    }
    audio.finish()?;
    if let Some(play) = game.world_play.as_ref() {
        let mut renderer = tantalus_client::world_view::WorldRenderer::default();
        let frame = renderer.render(play, width, height);
        tantalus_client::battle_view::save_png(&frame, &output)?;
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
    if std::env::var_os("TANTALUS_PERF").is_some() {
        play_perf(&game, &models, &field.scene.camera);
    }
    let mut drawn = tantalus_client::track::Drawn::default();
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
            }
            tantalus_client::scene_adapter::apply_overlay_changes(
                world,
                &field.scene,
                &game.world().overlays,
                tantalus_client::live::player_position(&frame),
            );
            let at = view.view(
                &game.world().camera_log,
                &field.scene.camera,
                field.scene.camera_index,
                tantalus_client::live::player_position(&frame),
                game.ticks,
            );
            tantalus_client::live::place_camera(world, at);
            if let Some((_, state, _)) = &motion {
                state.place_sprites(world, &field.scene);
            }
            tantalus_client::dialog_view::sync_dialogs(world, &game, &field.scene.camera, &frame);
            tantalus_client::menu_view::sync_menu(world, &game);
        },
    )
    .map_err(|error| error.to_string())?;
    let player = game
        .world()
        .player()
        .and_then(|player| game.world().actors.get(&player))
        .map(|actor| actor.position);
    Ok(format!(
        "saved field {} ({}) after {} ticks with {} actors ({} as models), player at {player:?}, to {}",
        game.map().unwrap_or(-1),
        game.field_name().unwrap_or("-"),
        game.ticks,
        drawn.actors,
        drawn.models,
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
        Ok(Self { log, wav })
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

/// `capture-world <play flags> [--world-at X,Z] --ticks N [--hold ...]
/// [--shots T,T] --output <png> [--size WxH]`: walks the world map
/// headlessly with scripted input and renders what the world camera sees.
fn run_world_capture(args: &[std::ffi::OsString]) -> ExitCode {
    let mut allowed = PLAY_FLAGS.to_vec();
    allowed.extend([
        "--ticks",
        "--hold",
        "--output",
        "--size",
        "--world-at",
        "--arrive-from",
        "--shots",
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
    let holds = parse_holds(values.get("--hold"))?;
    let shots: Vec<u64> = values
        .get("--shots")
        .map(|list| list.split(',').filter_map(|t| t.parse().ok()).collect())
        .unwrap_or_default();
    let (x, z) = match values.get("--world-at") {
        Some(at) => at
            .split_once(',')
            .and_then(|(x, z)| Some((x.parse::<f32>().ok()?, z.parse::<f32>().ok()?)))
            .ok_or("--world-at wants X,Z")?,
        None => (1218.0, -747.0),
    };
    let mut game = load_game(values)?;
    if let Some(from) = values.get("--arrive-from") {
        // Arrive as if leaving field `from` for the world map.
        let from: i32 = from.parse().map_err(|_| "bad --arrive-from")?;
        let mut events = Vec::new();
        game.start_world(Some(from), &mut events)?;
        for event in &events {
            println!("{}", tantalus_client::live::describe(event));
        }
    } else {
        game.begin_world(x, z)?;
    }
    let mut renderer = tantalus_client::world_view::WorldRenderer::default();
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
        "world at ({:.1}, {:.1}, {:.1}) yaw {:.1} camera {:.1} block {:?} ground id {} place {:?} danger steps {}; {} problems; wrote {}",
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
