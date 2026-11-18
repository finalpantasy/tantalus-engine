//! `capture-battle`: plays like `capture-play` until the first battle the
//! game fights (or `--battle N`, the Nth), then saves its battle view at
//! each of `--at SECONDS,...` into the replay, with no window. `--camera N`
//! overrides the pattern's default camera. `--window-frame <png>` also
//! renders the play window's composition offscreen: the field the game is
//! on, with the battle view opened over it the way `live` opens it.
//!
//! `--live 1` fights the first battle in real time instead, as the play
//! window does, with a scripted pad on the command menu, and saves
//! `<output>-menu`, `-list`, `-target`, `-attack`, `-hit` and `-end` PNGs
//! (the menu open, the ability list, the target cursor, the first party
//! attack mid-swing, its damage number, the result), plus `--at` battle
//! seconds as in a replay.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tantalus_client::battle_view::{BattleAssets, BattlePlayback, LiveBattle, save_png};
use tantalus_tools::battle_stage::BATTLE_TICKS_PER_SECOND;
use tantalus_tools::play::{PlayEvent, pad};

pub fn run(args: &[std::ffi::OsString]) -> ExitCode {
    // Battle and world frames draw on the GPU (`TANTALUS_RASTER_CPU=1`: the CPU).
    tantalus_client::frame_gpu::enable_headless();
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
        "--live",
        "--party-hp",
        "--recast",
        "--trance",
        "--status",
        "--enter-trance",
        "--party",
        "--bench-hz",
        "--bench-interp",
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
    tantalus_client::widescreen::set_target(width, height);
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
    let live = values.get("--live").is_some_and(|value| value != "0");
    let mut game = super::load_game(values)?;
    game.live_battles = live;
    // With `--live 1`, the frame-rate bench's battle
    // (`TANTALUS_BENCH_SCENE=battle:ID`) starts at once, for captures
    // beside the bench.
    if let Some(party) = values.get("--party") {
        let ids: Vec<i32> = party.split(',').filter_map(|id| id.parse().ok()).collect();
        game.set_party(&ids);
    }
    let mut bench_seen = 0usize;
    // `TANTALUS_BENCH_SCENE=battle:ID` also starts the replay's battle at
    // once, so effects can be checked without a route to a battle.
    if let Some(id) = std::env::var("TANTALUS_BENCH_SCENE")
        .ok()
        .and_then(|scene| scene.strip_prefix("battle:")?.parse::<i32>().ok())
    {
        let events = if live {
            game.bench_battle(id)
        } else {
            game.bench_fight(id)
        };
        for event in events {
            println!("{}", tantalus_client::live::describe(&event));
            if matches!(event, PlayEvent::Battle { .. }) {
                bench_seen += 1;
            }
        }
    }
    let mut open_for = 0u64;
    let mut last_confirm = false;
    let mut seen = bench_seen;
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
        if live && game.battle().is_some() {
            break;
        }
        if seen >= wanted && game.last_battle.is_some() {
            stage = game.last_battle.take();
            break;
        }
        if game.stopped.is_some() {
            break;
        }
    }
    if live {
        return live_capture(values, &mut game, &output, (width, height), &times);
    }
    let mut stage = stage.ok_or_else(|| {
        format!(
            "no battle {wanted} with a stage record in {} ticks",
            game.ticks
        )
    })?;
    // `--recast N:ABILITY` or `N:iITEM`: draws the stage's Nth party
    // command (from 0) as Black Magic ABILITY or as using ITEM, a preview
    // of effects the stand-in policy never casts. `N:ABILITYs` plays a
    // summon's short effect. The HP change and the end of the command move
    // to the new effect's timing (`EffectTable::command_timings`); the
    // numbers and the rest of the battle stay the original's.
    let party = stage
        .units
        .iter()
        .take_while(|unit| {
            matches!(
                unit.model,
                tantalus_tools::battle_stage::StageModel::Player { .. }
            )
        })
        .count();
    let mut recasts = std::collections::BTreeMap::new();
    for pair in values.get("--recast").map_or("", String::as_str).split(',') {
        if pair.is_empty() {
            continue;
        }
        let parsed = pair.split_once(':').and_then(|(index, to)| {
            let index = index.parse::<usize>().ok()?;
            Some(match to.strip_prefix('i') {
                Some(item) => (index, (14u32, item.parse::<i32>().ok()?, false)),
                None => match to.strip_suffix('s') {
                    Some(short) => (index, (22u32, short.parse::<i32>().ok()?, true)),
                    None => (index, (22u32, to.parse::<i32>().ok()?, false)),
                },
            })
        });
        let (index, to) = parsed
            .ok_or_else(|| format!("bad --recast entry `{pair}`, want N:ABILITY or N:iITEM"))?;
        recasts.insert(index, to);
    }
    let table = stage.effects.clone();
    let mut seen_commands = 0usize;
    // (unit, command tick, new effect tick, new end tick)
    let mut moves = Vec::new();
    for (tick, frame) in stage.frames.iter_mut().enumerate() {
        for event in &mut frame.events {
            if let tantalus_tools::battle_stage::StageEvent::Command {
                unit,
                cmd_no,
                sub_no,
                targets,
                short_summon,
                effect_in,
            } = event
                && *unit < party
            {
                if let Some((to_cmd, to_sub, short)) = recasts.get(&seen_commands) {
                    *cmd_no = *to_cmd;
                    *sub_no = *to_sub;
                    *short_summon = *short;
                    let variant = u8::from(targets.len() == 1) | (u8::from(*short) << 1);
                    if let Some((effect, total)) =
                        table.command_timing_of(&(*to_cmd == 14, *to_sub, variant))
                    {
                        moves.push((*unit, tick, effect as usize, total as usize));
                        // The moved `Effect` event places it now.
                        *effect_in = None;
                    }
                }
                seen_commands += 1;
            }
        }
    }
    for (unit, start, effect, total) in moves {
        move_command_events(&mut stage.frames, unit, start, effect, total);
    }
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
    for effect in tantalus_client::battle_effects::stage_effects(&playback.stage) {
        lines.push(format!(
            "effect {} by unit {} on {:?}: starts at {:.2}s, hits at {:.2}s, numbers at {:.2}s, HP changes at {:.2}s",
            effect.id,
            effect.caster,
            effect.targets,
            effect.start / BATTLE_TICKS_PER_SECOND,
            (effect.start + f64::from(effect.timing.hit)) / BATTLE_TICKS_PER_SECOND,
            (effect.start + f64::from(effect.timing.figure)) / BATTLE_TICKS_PER_SECOND,
            effect.applied / BATTLE_TICKS_PER_SECOND,
        ));
    }
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

/// A file next to `output` with `-<name>` before the extension.
fn named(output: &Path, name: &str) -> PathBuf {
    let stem = output
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    output.with_file_name(format!("{stem}-{name}.png"))
}

/// Scripted presses: `(seconds to wait, pad bit, shot to take first)`.
type Steps = Vec<(f64, i32, Option<&'static str>)>;

/// The presses for one menu. The first menu tours the windows for the
/// shots: the command window, the ability list, then Attack and its
/// target cursor; later menus attack the default target.
fn menu_steps(count: usize, ready: usize) -> Steps {
    match count {
        // The first menu: the window, Triangle to the next ready member,
        // the side column (Defend), then Steal on the first enemy.
        0 => {
            let mut steps = vec![(0.8, 0, Some("menu"))];
            if ready > 1 {
                steps.push((0.1, pad::TRIANGLE, None));
                steps.push((0.6, 0, Some("switch")));
            }
            steps.extend([
                (0.1, pad::RIGHT, None),
                (0.6, 0, Some("side")),
                (0.1, pad::LEFT, None),
                (0.3, pad::DOWN, None),
                (0.3, pad::CONFIRM, None),
                (0.6, pad::CONFIRM, Some("steal-target")),
            ]);
            steps
        }
        // The second: the item list, back out, then Attack.
        1 => vec![
            (0.5, pad::UP, None),
            (0.3, pad::CONFIRM, None),
            (0.6, pad::CANCEL, Some("list")),
            (0.3, pad::DOWN, None),
            (0.3, pad::CONFIRM, None),
            (0.6, pad::CONFIRM, Some("target")),
        ],
        // The third: Change (the side column's lower row) on oneself.
        2 => vec![
            (0.5, pad::RIGHT, None),
            (0.3, pad::DOWN, None),
            (0.3, pad::CONFIRM, None),
            (0.3, pad::CONFIRM, None),
        ],
        _ => vec![(0.3, pad::CONFIRM, None), (0.3, pad::CONFIRM, None)],
    }
}

/// The presses for `--party-hp`: Defend (the side column) every turn,
/// shooting the first menu.
fn defend_steps(count: usize) -> Steps {
    let first = if count == 0 { Some("menu") } else { None };
    vec![
        (0.8, 0, first),
        (0.1, pad::RIGHT, None),
        (0.3, pad::CONFIRM, None),
    ]
}

/// Seconds from a party Attack's start to the swing and to its number
/// (`battle_motion`: the blow is centered on the effect, 40 ticks in).
const SWING_SECONDS: f64 = 2.55;
const NUMBER_SECONDS: f64 = 3.0;

/// Fights the battle `game` has open in real time at 30 frames a second.
fn live_capture(
    values: &std::collections::BTreeMap<String, String>,
    game: &mut tantalus_tools::play::PlayGame,
    output: &Path,
    size: (u32, u32),
    times: &[f64],
) -> Result<Vec<String>, String> {
    use tantalus_client::battle_hud::BattleHud;
    use tantalus_client::battle_view::FightView;
    use tantalus_tools::battle_stage::StageEvent;
    let mut lines = Vec::new();
    let live = tantalus_client::widescreen::live_size();
    if size != live {
        lines.push(format!(
            "live frames are {}x{}; --size is ignored",
            live.0, live.1
        ));
    }
    let mut assets = BattleAssets::new(Path::new(&values["--assets"]));
    let hud = match BattleHud::load(game.game_data()) {
        Ok(hud) => Some(hud),
        Err(error) => {
            lines.push(format!("!! battle font and window art: {error}"));
            None
        }
    };
    let mut view = FightView::start(None, &mut assets, hud.as_ref(), game)
        .ok_or("no battle is being fought")?;
    if let Some(camera) = values.get("--camera") {
        view.set_camera(&assets, game, camera.parse().map_err(|_| "bad --camera")?);
    }
    lines.push(format!(
        "live battle {} scene {}: {} models, {} background triangles",
        view.battle, view.scene_name, view.scene.models, view.scene.background_triangles
    ));
    // `--party-hp N`: the party starts at N HP or less and only defends,
    // so the fight ends on the game over screen.
    let party_hp: Option<u32> = values
        .get("--party-hp")
        .map(|value| value.parse().map_err(|_| "bad --party-hp"))
        .transpose()?;
    if let (Some(hp), Some(fight)) = (party_hp, game.battle_mut()) {
        fight.cap_party_hp(hp);
    }
    // `--trance N`: every member's Trance gauge is on and at N.
    if let Some(trance) = values.get("--trance") {
        let trance: u8 = trance.parse().map_err(|_| "bad --trance")?;
        if let Some(fight) = game.battle_mut() {
            fight.preset_trance(trance);
        }
    }
    // `--status UNIT:HEXMASK,...`: units start under those `BattleStatus` bits (shown
    // afflicted; the statuses' side effects are not run).
    if let Some(list) = values.get("--status") {
        for entry in list.split(',') {
            let (unit, mask) = entry
                .split_once(':')
                .and_then(|(unit, mask)| {
                    Some((
                        unit.parse::<usize>().ok()?,
                        u64::from_str_radix(mask.trim_start_matches("0x"), 16).ok()?,
                    ))
                })
                .ok_or_else(|| format!("bad --status entry `{entry}`, want UNIT:HEXMASK"))?;
            if let Some(fight) = game.battle_mut() {
                fight.preset_status(unit, mask);
            }
        }
    }
    // `TANTALUS_BENCH_FRAMES=N --bench-hz H`: N display frames at H Hz of
    // the live battle (idle menu), main-thread milliseconds, no window.
    if let Some(frames) = tantalus_client::live::bench::frames() {
        use tantalus_client::live::bench;
        let hz: f64 = values
            .get("--bench-hz")
            .and_then(|value| value.parse().ok())
            .unwrap_or(60.0);
        for frame_number in 0..frames + 60 {
            if frame_number == 60 {
                bench::reset();
            }
            let start = std::time::Instant::now();
            view.update(game, 1.0 / hz, 0);
            if game.battle().is_none() {
                break;
            }
            std::hint::black_box(view.render(&assets, hud.as_ref(), game));
            bench::record("frame_cpu", start.elapsed().as_secs_f64() * 1000.0);
        }
        println!("bench battle {hz} Hz (battle time is continuous, drawn every frame)");
        bench::report();
        return Ok(lines);
    }
    let delta = 1.0 / 30.0;
    let mut steps: Steps = Vec::new();
    let mut waited = 0.0;
    let mut menus = 0;
    let mut stole: Option<f64> = None;
    let mut changed: Option<f64> = None;
    let mut saved: Vec<&str> = Vec::new();
    let mut attack: Option<f64> = None;
    let mut at_index = 0;
    let mut clock = 0.0;
    let save = |view: &FightView, game: &tantalus_tools::play::PlayGame, path: PathBuf| {
        save_png(&view.render(&assets, hud.as_ref(), game), &path)
            .map(|()| format!("saved {}", path.display()))
    };
    // Four minutes of battle at most.
    let mut scheduled: Vec<((usize, i32, i64), f64)> = Vec::new();
    for _ in 0..(30 * 240) {
        let mut held = 0;
        let seconds = game.battle().map_or(0.0, |fight| fight.stage().duration());
        if view.menu.open() && steps.is_empty() {
            let ready = game
                .battle()
                .map_or(0, |fight| fight.machine().ready_players.len());
            steps = if party_hp.is_some() {
                defend_steps(menus)
            } else {
                menu_steps(menus, ready)
            };
            menus += 1;
            waited = 0.0;
        }
        if !view.menu.open() {
            steps.clear();
        }
        waited += delta;
        if let Some(&(wait, bit, shot)) = steps.first()
            && waited >= wait
        {
            if let Some(shot) = shot {
                lines.push(save(&view, game, named(output, shot))?);
                saved.push(shot);
            }
            held |= bit;
            steps.remove(0);
            waited = 0.0;
        }
        // A party member's first Attack: shoot the swing and the number.
        if attack.is_none()
            && let Some(fight) = game.battle()
        {
            let party = fight.party_len();
            attack = fight
                .stage()
                .frames
                .iter()
                .position(|frame| {
                    frame.events.iter().any(|event| {
                        matches!(event, StageEvent::Command { unit, cmd_no: 1, .. } if *unit < party)
                    })
                })
                .map(|index| index as f64 / 15.0);
        }
        if let Some(start) = attack
            && game.battle().is_some()
        {
            for (name, after) in [("attack", SWING_SECONDS), ("hit", NUMBER_SECONDS)] {
                if !saved.contains(&name) && seconds >= start + after {
                    lines.push(save(&view, game, named(output, name))?);
                    saved.push(name);
                }
            }
        }
        // A member entering Trance: the effect, the new model, and its
        // menu with the Trance commands.
        if let Some(fight) = game.battle() {
            let changes = fight.stage().trance_changes();
            if let Some(&(unit, start, _)) = changes.iter().find(|change| change.2) {
                let start = start as f64 / 15.0;
                for (name, after) in [("trance-effect", 2.0), ("trance-model", 6.0)] {
                    if !saved.contains(&name) && seconds >= start + after {
                        lines.push(save(&view, game, named(output, name))?);
                        saved.push(name);
                    }
                }
                if !saved.contains(&"trance-menu")
                    && view.menu.open()
                    && view.menu.unit == Some(unit)
                    && waited > 0.3
                {
                    lines.push(save(&view, game, named(output, "trance-menu"))?);
                    saved.push("trance-menu");
                    if let Some(menu) = fight.menu(unit) {
                        let titles: Vec<String> = menu
                            .commands
                            .iter()
                            .map(|command| match &command.kind {
                                tantalus_tools::field_battle::CommandKind::List(list) => format!(
                                    "{} {:?}",
                                    command.title,
                                    list.iter()
                                        .map(|entry| entry.label.as_str())
                                        .collect::<Vec<_>>()
                                ),
                                _ => command.title.clone(),
                            })
                            .collect();
                        lines.push(format!("unit {unit} in Trance: commands {titles:?}"));
                    }
                }
            }
        }
        // A party Change: the member a row step away, half a second after
        // its row flips.
        if changed.is_none()
            && let Some(fight) = game.battle()
        {
            let frames = &fight.stage().frames;
            let party = fight.party_len();
            changed = frames.first().and_then(|first| {
                frames
                    .iter()
                    .position(|frame| {
                        frame
                            .rows
                            .iter()
                            .take(party)
                            .ne(first.rows.iter().take(party))
                    })
                    .map(|index| index as f64 / 15.0)
            });
        }
        if let Some(at) = changed
            && !saved.contains(&"change")
            && game.battle().is_some()
            && seconds >= at + 0.5
        {
            lines.push(save(&view, game, named(output, "change"))?);
            saved.push("change");
            if let Some(fight) = game.battle() {
                let frames = &fight.stage().frames;
                let rows = |frame: Option<&tantalus_tools::battle_stage::StageFrame>| {
                    frame.map(|frame| frame.rows.clone()).unwrap_or_default()
                };
                lines.push(format!(
                    "rows at the start {:?}, now {:?}",
                    rows(frames.first()),
                    rows(frames.last())
                ));
            }
        }
        // Steal's message, half a second after it lands.
        if stole.is_none()
            && let Some(fight) = game.battle()
        {
            stole = fight
                .stage()
                .frames
                .iter()
                .position(|frame| {
                    frame
                        .events
                        .iter()
                        .any(|event| matches!(event, StageEvent::Stole { .. }))
                })
                .map(|index| index as f64 / 15.0);
        }
        if let Some(at) = stole
            && !saved.contains(&"steal")
            && game.battle().is_some()
            && seconds >= at + 0.5
        {
            lines.push(save(&view, game, named(output, "steal"))?);
            saved.push("steal");
        }
        if let Some(&at) = times.get(at_index)
            && seconds >= at
            && game.battle().is_some()
        {
            lines.push(save(&view, game, numbered(output, at_index))?);
            at_index += 1;
        }
        // `--enter-trance N`: member N enters Trance once the menu is up.
        if let Some(unit) = values.get("--enter-trance")
            && !saved.contains(&"entered")
            && view.menu.open()
        {
            let unit: usize = unit.parse().map_err(|_| "bad --enter-trance")?;
            if let Some(fight) = game.battle_mut() {
                fight.enter_trance(unit);
            }
            saved.push("entered");
        }
        let (events, done) = view.update(game, delta, held);
        lines.extend(events.iter().map(tantalus_client::live::describe));
        // Each effect's start as the view schedules it, once, and again
        // if it ever moves (a live effect started early, then restarted).
        if let Some(fight) = game.battle() {
            for effect in tantalus_client::battle_effects::stage_effects(fight.stage()) {
                let key = (effect.caster, effect.id, effect.applied.round() as i64);
                let start = effect.start;
                match scheduled.iter_mut().find(|entry| {
                    entry.0.0 == key.0 && entry.0.1 == key.1 && (entry.1 - start).abs() < 200.0
                }) {
                    Some(entry) if entry.1 != start || entry.0.2 != key.2 => {
                        lines.push(format!(
                            "effect {} of unit {} moved: start {} -> {}, applied {} -> {}",
                            key.1, key.0, entry.1, start, entry.0.2, key.2
                        ));
                        *entry = (key, start);
                    }
                    Some(_) => {}
                    None => {
                        lines.push(format!(
                            "effect {} of unit {} scheduled at stage {:.1}s: start tick {start}, applied tick {}",
                            key.1,
                            key.0,
                            fight.stage().duration(),
                            key.2
                        ));
                        scheduled.push((key, start));
                    }
                }
            }
        }
        clock += delta;
        if done {
            break;
        }
        // The result screen: shoot it once it is up, then close it.
        if game.battle().is_none() && !saved.contains(&"end") {
            for _ in 0..75 {
                view.update(game, delta, 0);
            }
            lines.push(save(&view, game, named(output, "end"))?);
            saved.push("end");
            if let Some(results) = &view.results {
                lines.push(format!("results: {results:?}"));
            }
            let (_, done) = view.update(game, delta, pad::CONFIRM);
            if done {
                break;
            }
        }
    }
    lines.push(format!(
        "after {clock:.1}s: {}",
        game.stopped.as_deref().unwrap_or("the field resumed")
    ));
    Ok(lines)
}

/// Moves `unit`'s first `EffectApplied` events and its `Finished` after
/// `start` to `start + effect` and `start + total` (a `--recast` preview).
fn move_command_events(
    frames: &mut [tantalus_tools::battle_stage::StageFrame],
    unit: usize,
    start: usize,
    effect: usize,
    total: usize,
) {
    use tantalus_tools::battle_stage::StageEvent;
    let mut effects = Vec::new();
    let mut finished = None;
    for frame in frames.iter_mut().skip(start) {
        let mut index = 0;
        while index < frame.events.len() {
            let take = match &frame.events[index] {
                StageEvent::Effect { caster, .. } => *caster == unit && finished.is_none(),
                StageEvent::Finished { unit: who, .. } => *who == unit && finished.is_none(),
                _ => false,
            };
            if take {
                let event = frame.events.remove(index);
                if matches!(event, StageEvent::Finished { .. }) {
                    finished = Some(event);
                } else {
                    effects.push(event);
                }
            } else {
                index += 1;
            }
        }
        if finished.is_some() {
            break;
        }
    }
    let last = frames.len().saturating_sub(1);
    let effect_at = (start + effect).min(last);
    let end_at = (start + total).min(last);
    frames[effect_at].events.extend(effects);
    if let Some(event) = finished {
        frames[end_at].events.push(event);
    }
}
