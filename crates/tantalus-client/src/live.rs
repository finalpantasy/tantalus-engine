//! The playable window: [`tantalus_tools::play::PlayGame`] driven by the
//! keyboard at the field's 30 ticks a second, over the current field's
//! background, with each actor drawn as a marker.
//!
//! Keys: arrows or WASD walk (and move a choice cursor), Enter, Space or
//! Z confirm, Backspace or X cancel, Q triangle, E square, Tab select,
//! P start. Dialog lines, battles and field changes print to the console
//! and the current line shows in the window title (the client has no font
//! yet). Battles are fought live in the battle view
//! ([`crate::battle_view::FightView`]): the arrows, Enter and Backspace
//! drive the command menu, and the field resumes when the battle ends.

use std::collections::BTreeSet;

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowResolution};
use tantalus_assets::pc::bundled_field::BundledField;
use tantalus_contracts::{LegacyFieldCamera, Point3};
use tantalus_game::field::FieldWorld;
use tantalus_tools::play::{PlayEvent, PlayGame, pad};
use tantalus_tools::play_audio::AudioCue;

use crate::field_assets::resolve_field_assets_in;
use crate::scene_adapter::{ClientError, field_sprites, spawn_field_scene};
use crate::track::{TrackFrame, spawn_frame_markers};
use crate::window::{FIELD_FRAME, find_field, fit_field_frame};

/// Shortest time between redraws of the software-drawn views: 30 a second
/// on a 30, 60 or 144 Hz display (a little under a frame at 30 Hz).
const REDRAW_INTERVAL: f64 = 0.030;

/// Field ticks per second (`FPS_FIELD`).
pub const TICKS_PER_SECOND: f64 = 30.0;

/// Keyboard to `EventInput` bits.
const KEYS: [(KeyCode, i32); 21] = [
    (KeyCode::ArrowUp, pad::UP),
    (KeyCode::KeyW, pad::UP),
    (KeyCode::ArrowDown, pad::DOWN),
    (KeyCode::KeyS, pad::DOWN),
    (KeyCode::ArrowLeft, pad::LEFT),
    (KeyCode::KeyA, pad::LEFT),
    (KeyCode::ArrowRight, pad::RIGHT),
    (KeyCode::KeyD, pad::RIGHT),
    (KeyCode::Enter, pad::CONFIRM | pad::CIRCLE),
    (KeyCode::Space, pad::CONFIRM | pad::CIRCLE),
    (KeyCode::KeyZ, pad::CONFIRM | pad::CIRCLE),
    (KeyCode::Backspace, pad::CANCEL | pad::CROSS),
    (KeyCode::KeyX, pad::CANCEL | pad::CROSS),
    (KeyCode::KeyQ, pad::TRIANGLE),
    (KeyCode::KeyE, pad::SQUARE),
    // The world map camera turns with L1 and R1.
    (KeyCode::BracketLeft, pad::L1),
    (KeyCode::BracketRight, pad::R1),
    (KeyCode::PageUp, pad::L1),
    (KeyCode::PageDown, pad::R1),
    (KeyCode::Tab, pad::SELECT),
    (KeyCode::KeyP, pad::START),
];

/// Held keys as pad bits.
pub fn pad_bits(keys: &ButtonInput<KeyCode>) -> i32 {
    KEYS.iter()
        .filter(|(key, _)| keys.pressed(*key))
        .fold(0, |bits, (_, bit)| bits | bit)
}

/// The world's actors as one marker frame: every actor placed somewhere
/// other than the origin. A height the scripts left unset (|y| at or past
/// 32768) takes the walkmesh height under the actor.
/// `FF9StateSystem.EventState.ScenarioCounter`: the first global word.
fn scenario_counter(world: &FieldWorld) -> u16 {
    match world.global_memory.bytes() {
        [low, high, ..] => u16::from_le_bytes([*low, *high]),
        _ => 0,
    }
}

pub fn world_frame(world: &FieldWorld, tick: u64) -> (TrackFrame, BTreeSet<u64>) {
    let navigation = world
        .navigation()
        .and_then(|mesh| tantalus_game::navigation::FieldNavigation::new(mesh).ok());
    let poses = world
        .actors
        .iter()
        .map(|(id, actor)| {
            let playing = actor.animation_frames > 0;
            // Field animations run at the 30 Hz logic rate: a playing `ANIM`
            // is on its own frame counter, loops run off the clock.
            let time = if playing {
                f32::from(actor.animation_frame) / TICKS_PER_SECOND as f32
            } else {
                tick as f32 / TICKS_PER_SECOND as f32
            };
            (
                id.get(),
                crate::actor_models::ActorPose {
                    actor: id.get(),
                    facing: actor.facing.rem_euclid(256) as u8,
                    facing_frac: 0.0,
                    model: actor.look.model,
                    animation: actor.look.current_moving(
                        playing,
                        actor.movement.is_some() || actor.input_walking,
                        actor.input_running,
                    ),
                    time,
                    visible: actor.flags & 1 != 0,
                    hidden_meshes: actor.look.hidden_meshes,
                    script: world.actor_shadows.get(id).copied().unwrap_or_default(),
                    short_hair: scenario_counter(world) >= 10300,
                    scale: world
                        .actor_presentation
                        .get(id)
                        .filter(|look| look.scale != [4096; 3])
                        .map(|look| look.scale.map(|value| value as f32 / 4096.0)),
                    floor: world
                        .navigation()
                        .and_then(|mesh| crate::field_light::actor_floor(mesh, actor.position)),
                },
            )
        })
        .collect();
    let actors: Vec<(u64, [f32; 3])> = world
        .actors
        .iter()
        .map(|(id, actor)| {
            let mut position = actor.position;
            if position[1].abs() >= 32768.0
                && let Some(navigation) = &navigation
                && let Ok(projection) = navigation.project(Point3 {
                    x: position[0].round() as i32,
                    y: 0,
                    z: position[2].round() as i32,
                })
            {
                position[1] = projection.position.y as f32;
            }
            (id.get(), position)
        })
        .collect();
    let placed = actors
        .iter()
        .filter(|(_, position)| position.iter().any(|value| *value != 0.0))
        .map(|(id, _)| *id)
        .collect();
    let frame = TrackFrame {
        tick,
        player: world.player().map(|player| player.get()),
        actors,
        poses,
        attachments: world
            .attachments
            .iter()
            .filter_map(|(carried, held)| {
                Some((carried.get(), held.carrier?.get(), held.bone, held.offset))
            })
            .collect(),
    };
    (frame, placed)
}

/// Moves the 2D camera to follow the player (the frame's controlled
/// actor), kept inside the field camera's scroll range.
/// The player's position in `frame`, if it has one.
pub fn player_position(frame: &TrackFrame) -> Option<[f32; 3]> {
    let player = frame.player?;
    frame
        .actors
        .iter()
        .find(|(id, _)| *id == player)
        .map(|(_, position)| *position)
}

/// Feeds the view the camera requests logged this tick.
pub fn observe_view(view: &mut crate::view_scroll::ViewScroll, game: &PlayGame) {
    let key = format!(
        "{}:{}",
        game.map().unwrap_or(-1),
        game.field_name().unwrap_or("-")
    );
    view.observe(&key, game.world().camera_log.len(), game.ticks);
}

/// Centers the 2D camera on `at`, when there is a place to look.
/// Tells the widescreen view which field and camera is up (and the story
/// state its narrow scenes read) before the scroll range is asked for.
pub fn note_field_view(game: &PlayGame) {
    crate::widescreen::set_field(
        game.map().unwrap_or(-1),
        i32::from(game.world().field_camera),
        crate::widescreen::SceneState::from_globals(game.world().global_memory.bytes()),
    );
}

pub fn place_camera(world: &mut World, at: Option<[f32; 2]>) {
    let Some([x, y]) = at else {
        return;
    };
    let mut cameras = world.query_filtered::<&mut Transform, With<Camera2d>>();
    for mut transform in cameras.iter_mut(world) {
        transform.translation.x = x;
        transform.translation.y = y;
    }
}

pub fn follow_player(world: &mut World, camera: &LegacyFieldCamera, frame: &TrackFrame) {
    // `SceneService3DScroll` aims `charAimHeight` above the player.
    let target = frame
        .player
        .and_then(|player| frame.actors.iter().find(|(id, _)| *id == player))
        .and_then(|(_, position)| {
            crate::projection::project_field_point(
                camera,
                [
                    position[0],
                    position[1] + crate::scene_adapter::CHAR_AIM_HEIGHT,
                    position[2],
                ],
            )
        });
    let Some(target) = target else {
        return;
    };
    let [x, y] = crate::projection::follow(camera, target);
    let mut cameras = world.query_filtered::<&mut Transform, With<Camera2d>>();
    for mut transform in cameras.iter_mut(world) {
        transform.translation.x = x;
        transform.translation.y = y;
    }
}

/// Prints one event the way the console shows it.
pub fn describe(event: &PlayEvent) -> String {
    match event {
        PlayEvent::Field { map, name, .. } => format!("== field {map} {name}"),
        PlayEvent::Dialog { line, choice, .. } => {
            if *choice {
                format!("   {line}   [choice: up/down, then confirm]")
            } else {
                format!("   {line}")
            }
        }
        PlayEvent::Battle { battle, outcome } => format!("** battle {battle}: {outcome}"),
        PlayEvent::WorldMap {
            world_map,
            location,
            next,
        } => format!("** world map {world_map} location {location:?} -> {next:?}"),
        PlayEvent::Script { entry, tag } => format!("   (entry {entry} function {tag})"),
        PlayEvent::WorldWalk { place } => format!("** walking the world map from place {place}"),
        PlayEvent::DiscChange { field, disc } => {
            format!("** disc change: disc {} starts at field {field}", disc + 1)
        }
        PlayEvent::Stopped(reason) => format!("!! stopped: {reason}"),
    }
}

#[derive(Resource)]
struct Live {
    game: std::sync::Mutex<PlayGame>,
    streaming_assets: std::path::PathBuf,
    background: Option<String>,
    field: Option<tantalus_contracts::FieldSceneDescription>,
    /// The loaded field, kept to re-show it when the script switches
    /// cameras.
    bundled: Option<BundledField>,
    scene: Vec<Entity>,
    elapsed: f64,
    models: crate::actor_models::ActorModels,
    view: crate::view_scroll::ViewScroll,
    /// Game tick the actor markers were last drawn for; they are redrawn
    /// only when the tick or the shown field changes, not every frame.
    markers_at: Option<u64>,
    /// The field effect files (`FieldSPS`), read on first use.
    sps: tantalus_assets::pc::field_sps::FieldSpsLibrary,
    /// Scrolling background layers (`BGLLOOP`, `BGLPARALLAX`, ...).
    motion: Option<crate::overlay_motion::OverlayMotionState>,
    /// The battle view, while a battle is fought.
    fight: Option<crate::battle_view::FightView>,
    /// The world map view, while the player walks the world.
    world_view: Option<crate::world_view::LiveWorld>,
    battle_assets: crate::battle_view::BattleAssets,
    /// The battle HUD's font and window art, loaded with the first battle
    /// (`None` inside when they failed to load).
    battle_hud: std::sync::Mutex<Option<Option<crate::battle_hud::BattleHud>>>,
    /// Sound output; `None` with `--mute`.
    audio: Option<crate::audio::LiveAudio>,
    /// The cutscene movie on screen, with the pad bits held when it began or last changed.
    movie: Option<(crate::movie_view::MoviePlayer, i32)>,
    /// The Tetra Master game on screen.
    card: Option<crate::card_view::CardView>,
    /// Seconds since a software-drawn view (world map, battle, cards) last redrew.
    render_gap: f64,
    /// The frame rate setting (`--fps-cap`).
    fps: crate::smooth::FpsSettings,
    /// The last two logic frames of the field's actors, drawn blended.
    frames: crate::smooth::Pair<TrackFrame>,
    /// Actors placed somewhere, for the frame in `frames`.
    placed: BTreeSet<u64>,
    /// The last two logic positions of the field view's center.
    views: crate::smooth::Pair<[f32; 2]>,
    /// The field's SPS effects on the last two logic ticks.
    sps_slots: crate::smooth::Pair<Vec<tantalus_game::field::sps::SpsSlot>>,
    /// Share of the tick the field markers were last drawn at.
    drawn_alpha: f32,
    /// The last two logic poses of the world map's player and camera.
    world_poses: crate::smooth::Pair<crate::smooth::WorldPose>,
    /// Tick of the last world pose.
    world_pose_at: Option<u64>,
}

/// Opens the play window. `game` is already on its first field.
pub fn play(
    mut game: PlayGame,
    streaming_assets: std::path::PathBuf,
    scale: u32,
    mute: bool,
    fps: crate::smooth::FpsSettings,
) -> Result<(), ClientError> {
    let scale = scale.clamp(1, 8);
    game.live_battles = true;
    game.live_movies = true;
    let audio =
        (!mute).then(|| crate::audio::LiveAudio::start(&streaming_assets, game.game_data()));
    crate::dialog_view::preload(game.game_data());
    let models = crate::actor_models::ActorModels::new(&streaming_assets);
    let streaming_assets_for_sps = streaming_assets.clone();
    let battle_assets = crate::battle_view::BattleAssets::new(&streaming_assets);
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb_u8(18, 24, 36)))
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Tantalus Engine".into(),
                        present_mode: fps.present_mode(),
                        resolution: WindowResolution::new(
                            FIELD_FRAME.0 as u32 * scale,
                            FIELD_FRAME.1 as u32 * scale,
                        ),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(crate::psx_blend::PsxBlendPlugin)
        .add_plugins(crate::actor_gpu::ActorGpuPlugin)
        .add_plugins(crate::frame_gpu::FrameGpuPlugin)
        .insert_resource(Live {
            game: std::sync::Mutex::new(game),
            streaming_assets,
            background: None,
            field: None,
            bundled: None,
            scene: Vec::new(),
            elapsed: 0.0,
            models,
            view: crate::view_scroll::ViewScroll::default(),
            markers_at: None,
            sps: crate::sps_view::library(&streaming_assets_for_sps),
            motion: None,
            fight: None,
            world_view: None,
            battle_assets,
            battle_hud: std::sync::Mutex::new(None),
            movie: None,
            card: None,
            audio,
            render_gap: 0.0,
            fps,
            frames: crate::smooth::Pair::default(),
            placed: BTreeSet::new(),
            views: crate::smooth::Pair::default(),
            sps_slots: crate::smooth::Pair::default(),
            drawn_alpha: -1.0,
            world_poses: crate::smooth::Pair::default(),
            world_pose_at: None,
        })
        .insert_resource(FrameCap(crate::smooth::FrameLimiter::new(
            fps.frame_budget(),
        )))
        .add_systems(Update, live_tick)
        .add_systems(Last, hold_frame_cap);
    app.run();
    Ok(())
}

/// The `--fps-cap N` limiter.
#[derive(Resource)]
struct FrameCap(crate::smooth::FrameLimiter);

/// Ends each frame by waiting out the rest of its cap budget.
fn hold_frame_cap(mut cap: ResMut<FrameCap>) {
    cap.0.wait();
}

/// Puts `field`'s background and camera in `world`, replacing `old`.
pub fn show_field(
    world: &mut World,
    field: &BundledField,
    old: &mut Vec<Entity>,
) -> Result<(), ClientError> {
    for entity in old.drain(..) {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
    let assets = resolve_field_assets_in(world, field)?;
    let sprites = field_sprites(&field.scene, &assets)?;
    old.extend(spawn_field_scene(
        world,
        &field.scene,
        &sprites,
        &assets,
        None,
    ));
    fit_field_frame(world);
    Ok(())
}

/// Runs the layer scroll service for the tick `game` just played.
fn step_overlay_motion(
    field: &Option<tantalus_contracts::FieldSceneDescription>,
    background: Option<&str>,
    motion: &mut Option<crate::overlay_motion::OverlayMotionState>,
    view: &mut crate::view_scroll::ViewScroll,
    game: &PlayGame,
) {
    let (Some(scene), Some(motion)) = (field, motion) else {
        return;
    };
    if game.background() != background {
        return;
    }
    let at = if motion.needs_camera() {
        // The scroll range reads the widescreen view of this field.
        note_field_view(game);
        let (frame, _) = world_frame(game.world(), game.ticks);
        view.view(
            &game.world().camera_log,
            &scene.camera,
            scene.camera_index,
            player_position(&frame),
            game.ticks,
        )
    } else {
        None
    };
    motion.map = game.map().unwrap_or(-1);
    motion.tick(&game.world().overlay_motion, at);
}

/// Hands the sound requests a game step made (script sounds, menu presses, battle songs) to the
/// audio thread.
fn send_audio(audio: Option<&crate::audio::LiveAudio>, game: &mut PlayGame) {
    let cues: Vec<AudioCue> = game.take_audio().into_iter().map(|(_, cue)| cue).collect();
    if let Some(audio) = audio {
        audio.cues(cues);
    }
}

/// Frame timing for the play window, on when `TANTALUS_BENCH_FRAMES=N` is
/// set: per-section milliseconds over N frames, printed once, then exit.
pub mod bench {
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::time::Instant;

    static SAMPLES: Mutex<BTreeMap<&'static str, Vec<f64>>> = Mutex::new(BTreeMap::new());

    pub fn frames() -> Option<usize> {
        static FRAMES: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
        *FRAMES.get_or_init(|| {
            std::env::var("TANTALUS_BENCH_FRAMES")
                .ok()
                .and_then(|value| value.parse().ok())
        })
    }

    /// Drops what was recorded (a headless bench's warm-up frames).
    pub fn reset() {
        SAMPLES.lock().expect("bench lock").clear();
    }

    pub fn record(name: &'static str, ms: f64) {
        if frames().is_some() {
            SAMPLES
                .lock()
                .expect("bench lock")
                .entry(name)
                .or_default()
                .push(ms);
        }
    }

    /// Runs `body`, recording its time under `name`.
    pub fn time<T>(name: &'static str, body: impl FnOnce() -> T) -> T {
        if frames().is_none() {
            return body();
        }
        let start = Instant::now();
        let out = body();
        record(name, start.elapsed().as_secs_f64() * 1000.0);
        out
    }

    pub fn report() {
        let samples = SAMPLES.lock().expect("bench lock");
        println!("bench section: n mean p50 p95 max (ms)");
        for (name, values) in samples.iter() {
            let mut sorted = values.clone();
            sorted.sort_by(|a, b| a.total_cmp(b));
            let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q) as usize];
            let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
            println!(
                "bench {name}: {} {mean:.2} {:.2} {:.2} {:.2}",
                sorted.len(),
                at(0.5),
                at(0.95),
                sorted[sorted.len() - 1]
            );
        }
    }
}

fn live_tick(world: &mut World) {
    let Some(limit) = bench::frames() else {
        live_tick_inner(world);
        return;
    };
    bench::record(
        "frame_interval",
        world.resource::<Time>().delta_secs_f64() * 1000.0,
    );
    bench::time("live_tick", || live_tick_inner(world));
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let count = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    // `TANTALUS_BENCH_SHOT=out.png`: a screenshot of the window after the
    // last timed frame, a few frames before the exit so it can save.
    let shot = std::env::var("TANTALUS_BENCH_SHOT").ok();
    if count == limit {
        bench::report();
        if let Some(path) = &shot {
            use bevy::render::view::screenshot::{Screenshot, save_to_disk};
            world
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path.clone()));
        }
    }
    if count >= limit + if shot.is_some() { 20 } else { 0 } {
        std::process::exit(0);
    }
}

/// The play window's pixel size, for the widescreen view (`--aspect auto`
/// follows it).
fn note_window_size(world: &mut World) {
    let mut windows = world.query_filtered::<&Window, With<PrimaryWindow>>();
    if let Ok(window) = windows.single(world) {
        crate::widescreen::set_target(window.physical_width(), window.physical_height());
    }
}

fn live_tick_inner(world: &mut World) {
    let delta = world.resource::<Time>().delta_secs_f64();
    let held = pad_bits(world.resource::<ButtonInput<KeyCode>>());
    note_window_size(world);
    let mut live = world.remove_resource::<Live>().expect("live state");
    // The software-drawn views change at most 30 times a second, so a
    // faster display redraws them only that often: the redraw is the costly
    // part of a frame, and skipping it leaves the tick untouched.
    live.render_gap += delta;
    let redraw = live.render_gap >= REDRAW_INTERVAL;
    // The GPU-drawn battle and world views cost little to record, so they
    // redraw every display frame.
    let gpu_redraw = redraw || crate::frame_gpu::active();
    if let Some((mut player, before)) = live.movie.take() {
        // Start skips; a Start held since before the movie began does not.
        let skipped = held & pad::START != 0 && before & pad::START == 0;
        if player.update(world) || skipped {
            player.close(world);
            let mut game = live.game.lock().expect("game lock");
            game.end_movie();
            game.movie_sound(None);
            send_audio(live.audio.as_ref(), &mut game);
            drop(game);
            live.elapsed = 0.0;
        } else {
            live.movie = Some((player, before));
            world.insert_resource(live);
            return;
        }
    }
    if let Some(mut view) = live.fight.take() {
        let mut game = live.game.lock().expect("game lock");
        {
            let hud = live.battle_hud.lock().expect("hud lock");
            view.resize_if_stale(
                world,
                &live.battle_assets,
                hud.as_ref().and_then(Option::as_ref),
                &game,
            );
        }
        let (events, done) = bench::time("battle_update", || view.update(&mut game, delta, held));
        for event in &events {
            println!("{}", describe(event));
        }
        send_audio(live.audio.as_ref(), &mut game);
        if done {
            view.finish(world);
            live.elapsed = 0.0;
        } else {
            let hud = live.battle_hud.lock().expect("hud lock");
            if gpu_redraw {
                live.render_gap = 0.0;
                bench::time("battle_show", || {
                    view.show(
                        world,
                        &live.battle_assets,
                        hud.as_ref().and_then(Option::as_ref),
                        &game,
                    )
                });
            }
            crate::battle_view::set_title(world, view.title());
            drop(hud);
            drop(game);
            live.fight = Some(view);
            world.insert_resource(live);
            return;
        }
    }
    live.elapsed += delta;
    let step = 1.0 / TICKS_PER_SECOND;
    let mut steps = 0;
    {
        let mut game = live.game.lock().expect("game lock");
        while live.elapsed >= step && steps < 4 {
            live.elapsed -= step;
            steps += 1;
            match bench::time("game_tick", || game.tick(held)) {
                Ok(events) => {
                    for event in &events {
                        println!("{}", describe(event));
                    }
                }
                Err(error) => {
                    println!("!! tick failed: {error}");
                    game.stopped = Some(error);
                }
            }
            send_audio(live.audio.as_ref(), &mut game);
            observe_view(&mut live.view, &game);
            step_overlay_motion(
                &live.field,
                live.background.as_deref(),
                &mut live.motion,
                &mut live.view,
                &game,
            );
            if game.battle().is_some() || game.movie().is_some() {
                break;
            }
        }
        if steps == 4 {
            live.elapsed = 0.0;
        }
        if game.battle().is_some() {
            live.elapsed = 0.0;
            let mut hud = live.battle_hud.lock().expect("hud lock");
            let hud = hud.get_or_insert_with(|| {
                crate::battle_hud::BattleHud::load(game.game_data())
                    .map_err(|error| println!("!! battle font and window art: {error}"))
                    .ok()
            });
            live.fight = bench::time("load_battle", || {
                crate::battle_view::FightView::start(
                    Some(world),
                    &mut live.battle_assets,
                    hud.as_ref(),
                    &game,
                )
            });
        }
    }
    {
        let mut game = live.game.lock().expect("game lock");
        if let Some(request) = game
            .movie()
            .filter(|_| live.movie.is_none() && live.fight.is_none())
        {
            match crate::movie_view::MoviePlayer::start(&request.path) {
                Ok(player) => {
                    println!("** movie {}", request.name);
                    live.movie = Some((player, held));
                    game.movie_sound(request.sound_resource());
                }
                Err(error) => {
                    println!("!! movie {}: {error}", request.name);
                    game.end_movie();
                }
            }
            send_audio(live.audio.as_ref(), &mut game);
        }
    }
    {
        let game = live.game.lock().expect("game lock");
        match game.card_game() {
            Some(card) => {
                let view = live
                    .card
                    .get_or_insert_with(|| crate::card_view::CardView::open(game.game_data()));
                if redraw || !view.is_open() {
                    live.render_gap = 0.0;
                    bench::time("card", || view.update(world, card));
                }
            }
            None => {
                if let Some(view) = live.card.take() {
                    view.close(world);
                }
            }
        }
    }
    // The battle view covers the field from here; leave the field as it is
    // until the battle ends.
    if live.fight.is_some() {
        world.insert_resource(live);
        return;
    }
    {
        let game = live.game.lock().expect("game lock");
        if let Some(play) = game.world_play.as_ref() {
            match live.world_view.as_mut() {
                Some(view) => {
                    // The logic pose is read once a tick; the frames between
                    // draw the last two blended.
                    if live.world_pose_at != Some(game.ticks) {
                        let snap = live.world_pose_at.is_none();
                        live.world_poses
                            .push(crate::world_view::pose_of(play), snap);
                        live.world_pose_at = Some(game.ticks);
                    }
                    if gpu_redraw {
                        live.render_gap = 0.0;
                        let t = crate::smooth::alpha(live.elapsed, live.fps.interpolate);
                        let pose = live.world_poses.world(t);
                        bench::time("world_map", || view.update(world, play, pose.as_ref()));
                    }
                }
                None => {
                    live.world_view = Some(bench::time("load_world", || {
                        crate::world_view::LiveWorld::start(world, play, &live.streaming_assets)
                    }))
                }
            }
            // The main menu, save screen and Chocograph list over the world map.
            bench::time("menu", || crate::menu_view::sync_menu(world, &game));
            drop(game);
            world.insert_resource(live);
            return;
        }
    }
    if let Some(view) = live.world_view.take() {
        view.finish(world);
        live.world_poses.clear();
        live.world_pose_at = None;
    }
    let mut game = live.game.lock().expect("game lock");
    let background = game.background().map(str::to_owned);
    if background != live.background {
        live.background = background.clone();
        live.field = None;
        live.bundled = None;
        live.markers_at = None;
        live.motion = None;
        if let Some(name) = &background {
            match bench::time("load_find_field", || {
                find_field(&live.streaming_assets, name)
            }) {
                Ok(field) => live.bundled = Some(field),
                Err(error) => println!("!! no background {name}: {error}"),
            }
        }
    }
    let camera = u32::from(game.world().field_camera);
    if let Some(field) = &mut live.bundled
        && live.field.as_ref().is_none_or(|shown| {
            shown.camera_index != camera && (camera as usize) < field.scene.cameras.len()
        })
    {
        if let Some(scene) = field.scene.with_camera(camera) {
            field.scene = scene;
        }
        let mut scene = std::mem::take(&mut live.scene);
        if let Err(error) = bench::time("load_show_field", || show_field(world, field, &mut scene))
        {
            println!("!! cannot show {}: {error}", field.name);
        }
        live.scene = scene;
        live.field = Some(field.scene.clone());
        game.set_sound_camera(Some(field.scene.camera.clone()));
        live.markers_at = None;
        match &mut live.motion {
            Some(motion) => motion.set_camera(camera as u8),
            None => {
                live.motion =
                    crate::overlay_motion::OverlayMotionState::new(&field.background, &field.scene);
            }
        }
    }
    if let Some(scene) = &live.field {
        // The logic state is read once a tick. Between ticks the display
        // frames draw the last two logic states blended (`smooth.rs`); the
        // game itself is only read, never written, from here on.
        let new_tick = live.markers_at != Some(game.ticks);
        if new_tick {
            let snap = live.markers_at.is_none();
            let (frame, placed) =
                bench::time("world_frame", || world_frame(game.world(), game.ticks));
            let field_name = game.field_name().map(str::to_owned);
            for problem in bench::time("load_prepare_field", || {
                live.models.prepare_field(field_name.as_deref())
            }) {
                println!("!! field light: {problem}");
            }
            live.models.apply_light_changes(&game.world().light_changes);
            for problem in live.models.prepare(frame.poses.values()) {
                println!("!! actor models: {problem}");
            }
            live.models.advance_actor_tex_anims(
                game.ticks,
                &frame.poses,
                &game.world().tex_anims,
                game.world().tex_anim_clock,
            );
            let at = live.view.view(
                &game.world().camera_log,
                &scene.camera,
                scene.camera_index,
                player_position(&frame),
                game.ticks,
            );
            if snap {
                live.views.clear();
            }
            if let Some(at) = at {
                live.views.push(at, snap);
            }
            live.frames.push(frame, snap);
            live.sps_slots.push(game.world().sps.slots.clone(), snap);
            live.placed = placed;
            live.markers_at = Some(game.ticks);
        }
        let t = crate::smooth::alpha(live.elapsed, live.fps.interpolate);
        let frame = live
            .frames
            .frame(t)
            .expect("a logic frame is captured before the first draw");
        if new_tick || (live.fps.interpolate && t != live.drawn_alpha) {
            bench::time("markers", || {
                spawn_frame_markers(
                    world,
                    &scene.camera,
                    &frame,
                    &live.placed,
                    Some(&live.models),
                )
            });
            let sps_slots = live.sps_slots.slots(t);
            bench::time("sps", || {
                crate::sps_view::sync_game_sps(
                    world,
                    &game,
                    &scene.camera,
                    &frame,
                    Some(&live.models),
                    &live.sps,
                    &sps_slots,
                )
            });
            live.drawn_alpha = t;
        }
        crate::scene_adapter::apply_overlay_changes(
            world,
            scene,
            &game.world().overlays,
            player_position(&frame),
        );
        crate::scene_adapter::apply_tile_shade(world, &game.world().tile_shade);
        note_field_view(&game);
        place_camera(world, live.views.view(t));
        fit_field_frame(world);
        if let Some(motion) = &mut live.motion
            && !motion.is_idle()
        {
            motion.place_sprites(world, scene, t);
        }
        bench::time("dialogs", || {
            crate::dialog_view::sync_dialogs(world, &game, &scene.camera, &frame)
        });
        crate::presentation_view::sync_presentation(
            world,
            game.world(),
            game.game_data(),
            &scene.camera,
            &frame,
            game.ticks,
        );
    }
    // Outside the field branch: field 70 (between discs) has no background to draw.
    bench::time("menu", || crate::menu_view::sync_menu(world, &game));
    crate::disc_view::sync_disc(world, &game);
    let mut title = format!(
        "Tantalus Engine: {} ({})",
        game.field_name().unwrap_or("-"),
        game.map().unwrap_or(-1)
    );
    if let Some((_, line)) = game.open_text().last() {
        title.push_str(" | ");
        title.push_str(line);
        if game.cursor > 0 {
            title.push_str(&format!(" [answer {}]", game.cursor + 1));
        }
    }
    if let Some(reason) = &game.stopped {
        title.push_str(&format!(" | stopped: {reason}"));
    }
    drop(game);
    let mut windows = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
    if let Ok(mut window) = windows.single_mut(world)
        && window.title != title
    {
        window.title = title;
    }
    world.insert_resource(live);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_pad_bits() {
        let mut keys = ButtonInput::<KeyCode>::default();
        assert_eq!(pad_bits(&keys), 0);
        keys.press(KeyCode::ArrowUp);
        keys.press(KeyCode::KeyD);
        keys.press(KeyCode::Enter);
        assert_eq!(
            pad_bits(&keys),
            pad::UP | pad::RIGHT | pad::CONFIRM | pad::CIRCLE
        );
    }

    #[test]
    fn world_frame_draws_placed_actors() {
        let mut world = FieldWorld::new();
        let placed = tantalus_contracts::EntityId::new(3);
        world.add_actor(placed, [10.0, 32768.0, 20.0]);
        world.add_actor(tantalus_contracts::EntityId::new(4), [0.0; 3]);
        let (frame, drawn) = world_frame(&world, 7);
        assert_eq!(frame.tick, 7);
        // No walkmesh: the height is left as the scripts set it.
        assert!(frame.actors.contains(&(3, [10.0, 32768.0, 20.0])));
        assert_eq!(drawn, BTreeSet::from([3]));
    }

    /// The smooth frame rate only reads the game: moving actors are drawn
    /// blended at any rate and the world they came from is the same after,
    /// and the 30 Hz logic frames are the same whatever the display rate.
    #[test]
    fn blending_never_feeds_back_into_the_game() {
        let mut world = FieldWorld::new();
        let id = tantalus_contracts::EntityId::new(3);
        world.add_actor(id, [0.0, 0.0, 0.0]);
        let mut pair = crate::smooth::Pair::default();
        let mut logic = Vec::new();
        for tick in 0..30u64 {
            let before = world.clone();
            world.actors.get_mut(&id).expect("actor").position[0] += 7.0;
            let (frame, _) = world_frame(&world, tick);
            logic.push(frame.clone());
            pair.push(frame, tick == 0);
            let at_tick = world.clone();
            // Draw 8 display frames of the tick (240 Hz on 30 Hz logic).
            for step in 0..8 {
                let drawn = pair.frame(step as f32 / 8.0).expect("frame");
                if tick > 0 {
                    let x = drawn.actors[0].1[0];
                    assert!(x >= before.actors[&id].position[0] - 1e-3);
                    assert!(x <= world.actors[&id].position[0] + 1e-3);
                }
            }
            assert!(world == at_tick, "drawing changed the world");
        }
        // The logic frames: the same run captured again, nothing blended in.
        let mut again = FieldWorld::new();
        again.add_actor(id, [0.0, 0.0, 0.0]);
        for (tick, frame) in logic.iter().enumerate() {
            again.actors.get_mut(&id).expect("actor").position[0] += 7.0;
            assert_eq!(&world_frame(&again, tick as u64).0, frame);
        }
    }
}
