//! `capture-battle`: plays like `capture-play` until the first battle the
//! game fights (or `--battle N`, the Nth), then saves its battle view at
//! each of `--at SECONDS,...` into the replay, with no window. `--camera N`
//! overrides the pattern's default camera. `--window-frame <png>` also
//! renders the play window's composition offscreen: the field the game is
//! on, with the battle view opened over it the way `live` opens it.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tantalus_client::battle_view::{BattleAssets, BattlePlayback, LiveBattle, save_png};
use tantalus_tools::play::{PlayEvent, pad};

pub fn run(args: &[std::ffi::OsString]) -> ExitCode {
    let mut allowed = super::PLAY_FLAGS.to_vec();
    allowed.extend([
        "--ticks",
        "--hold",
        "--auto-confirm",
        "--output",
        "--size",
        "--at",
        "--battle",
        "--camera",
        "--window-frame",
    ]);
    let Some(values) = super::flag_values(args, &allowed) else {
        return ExitCode::FAILURE;
    };
    match capture(&values) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("capture-battle failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `out.png` with `-<index>` before the extension for every time but the
/// first.
fn numbered(output: &Path, index: usize) -> PathBuf {
    if index == 0 {
        return output.to_path_buf();
    }
    let stem = output
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    output.with_file_name(format!("{stem}-{index}.png"))
}

fn capture(values: &std::collections::BTreeMap<String, String>) -> Result<Vec<String>, String> {
    let output = values
        .get("--output")
        .map(PathBuf::from)
        .ok_or("requires --output")?;
    let ticks: u64 = values
        .get("--ticks")
        .map_or(Ok(20_000), |value| value.parse().map_err(|_| "bad --ticks"))?;
    let auto_confirm: u64 = values.get("--auto-confirm").map_or(Ok(30), |value| {
        value.parse().map_err(|_| "bad --auto-confirm")
    })?;
    let wanted: usize = values
        .get("--battle")
        .map_or(Ok(1), |value| value.parse().map_err(|_| "bad --battle"))?;
    let (width, height) = match values.get("--size") {
        None => (640, 480),
        Some(size) => super::parse_size(Path::new(size)).ok_or("--size must be WxH")?,
    };
    let times: Vec<f64> = values
        .get("--at")
        .map_or("1", String::as_str)
        .split(',')
        .map(|value| {
            value
                .trim()
                .parse()
                .map_err(|_| format!("bad --at `{value}`"))
        })
        .collect::<Result<_, _>>()?;
    if values.get("--battles").is_none() {
        return Err("requires --battles (without it battles are skipped)".into());
    }
    let holds = super::parse_holds(values.get("--hold"))?;
    let mut game = super::load_game(values)?;
    let mut open_for = 0u64;
    let mut last_confirm = false;
    let mut seen = 0usize;
    let mut stage = None;
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
        if auto_confirm > 0 && open_for >= auto_confirm && !last_confirm {
            held |= pad::CONFIRM;
            open_for = 0;
        }
        last_confirm = held & pad::CONFIRM != 0;
        for event in game.tick(held)? {
            println!("{}", tantalus_client::live::describe(&event));
            if matches!(event, PlayEvent::Battle { .. }) {
                seen += 1;
            }
        }
        if seen >= wanted && game.last_battle.is_some() {
            stage = game.last_battle.take();
            break;
        }
        if game.stopped.is_some() {
            break;
        }
    }
    let mut stage = stage.ok_or_else(|| {
        format!(
            "no battle {wanted} with a stage record in {} ticks",
            game.ticks
        )
    })?;
    // `--camera N` shows another default camera than the pattern's.
    if let Some(camera) = values.get("--camera") {
        stage.camera = camera.parse().map_err(|_| "bad --camera")?;
    }
    let mut assets = BattleAssets::new(Path::new(&values["--assets"]));
    let mut lines = assets
        .prepare(&stage)
        .into_iter()
        .map(|problem| format!("!! battle assets: {problem}"))
        .collect::<Vec<_>>();
    let mut playback = BattlePlayback::new(&assets, stage, width, height);
    lines.push(format!(
        "battle {} scene {} pattern {} camera {}: {} units ({} with models), {} background triangles, {} ticks recorded, outcome {}",
        playback.stage.battle,
        playback.stage.scene,
        playback.stage.pattern,
        playback.stage.camera,
        playback.stage.units.len(),
        playback.scene.models,
        playback.scene.background_triangles,
        playback.stage.frames.len(),
        playback.stage.outcome.as_deref().unwrap_or("-"),
    ));
    for (index, at) in times.iter().enumerate() {
        playback.seconds = *at;
        let path = numbered(&output, index);
        save_png(&playback.render(&assets), &path)?;
        lines.push(format!("saved {at}s to {}", path.display()));
    }
    if let Some(window_frame) = values.get("--window-frame") {
        let background = game
            .background()
            .ok_or("the battle ended on no field background")?;
        let field = tantalus_client::window::find_field(Path::new(&values["--assets"]), background)
            .map_err(|error| error.to_string())?;
        let stage = playback.stage.clone();
        let at = times.first().copied().unwrap_or(1.0);
        tantalus_client::capture::capture_field_scene(
            &field,
            Path::new(window_frame),
            tantalus_client::capture::CaptureConfig {
                width,
                height,
                ..Default::default()
            },
            |world| {
                let mut live = LiveBattle::start(world, &mut assets, stage);
                live.tick(world, &assets, at, 0);
            },
        )
        .map_err(|error| error.to_string())?;
        lines.push(format!("saved the window frame to {window_frame}"));
    }
    Ok(lines)
}
