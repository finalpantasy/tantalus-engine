//! The playable window: [`tantalus_tools::play::PlayGame`] driven by the
//! keyboard at the field's 30 ticks a second, over the current field's
//! background, with each actor drawn as a marker.
//!
//! Keys: arrows or WASD walk (and move a choice cursor), Enter, Space or
//! Z confirm, Backspace or X cancel, Q triangle, E square, Tab select,
//! P start. Dialog lines, battles and field changes print to the console
//! and the current line shows in the window title (the client has no font
//! yet). A fought battle replays in the battle view
//! ([`crate::battle_view`]) before the field resumes.

use std::collections::BTreeSet;

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowResolution};
use tantalus_assets::pc::bundled_field::BundledField;
use tantalus_contracts::{LegacyFieldCamera, Point3};
use tantalus_game::field::FieldWorld;
use tantalus_tools::play::{PlayEvent, PlayGame, pad};

use crate::field_assets::resolve_field_assets_in;
use crate::scene_adapter::{ClientError, field_sprites, spawn_field_scene};
use crate::track::{TrackFrame, spawn_frame_markers};
use crate::window::{FIELD_FRAME, find_field, fit_field_frame};

/// Field ticks per second (`FPS_FIELD`).
pub const TICKS_PER_SECOND: f64 = 30.0;

/// Keyboard to `EventInput` bits.
const KEYS: [(KeyCode, i32); 17] = [
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
                    facing: actor.facing.rem_euclid(256) as u8,
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
    /// Scrolling background layers (`BGLLOOP`, `BGLPARALLAX`, ...).
    motion: Option<crate::overlay_motion::OverlayMotionState>,
    /// The battle view, while a fought battle replays.
    battle: Option<crate::battle_view::LiveBattle>,
    battle_assets: crate::battle_view::BattleAssets,
}

/// Opens the play window. `game` is already on its first field.
pub fn play(
    game: PlayGame,
    streaming_assets: std::path::PathBuf,
    scale: u32,
) -> Result<(), ClientError> {
    let scale = scale.clamp(1, 8);
    let models = crate::actor_models::ActorModels::new(&streaming_assets);
    let battle_assets = crate::battle_view::BattleAssets::new(&streaming_assets);
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb_u8(18, 24, 36)))
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Tantalus Engine".into(),
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
            motion: None,
            battle: None,
            battle_assets,
        })
        .add_systems(Update, live_tick);
    app.run();
    Ok(())
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
    motion.tick(&game.world().overlay_motion, at);
}

fn live_tick(world: &mut World) {
    let delta = world.resource::<Time>().delta_secs_f64();
    let held = pad_bits(world.resource::<ButtonInput<KeyCode>>());
    let mut live = world.remove_resource::<Live>().expect("live state");
    if crate::battle_view::run_live_battle(
        world,
        &mut live.battle,
        &live.battle_assets,
        delta,
        held,
    ) {
        world.insert_resource(live);
        return;
    }
    live.elapsed += delta;
    let step = 1.0 / TICKS_PER_SECOND;
    let mut steps = 0;
    {
        let mut game = live.game.lock().expect("game lock");
        while live.elapsed >= step && steps < 4 {
            live.elapsed -= step;
            steps += 1;
            match game.tick(held) {
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
            observe_view(&mut live.view, &game);
            step_overlay_motion(
                &live.field,
                live.background.as_deref(),
                &mut live.motion,
                &mut live.view,
                &game,
            );
            if game.last_battle.is_some() {
                break;
            }
        }
        if steps == 4 {
            live.elapsed = 0.0;
        }
        if let Some(stage) = game.last_battle.take() {
            live.elapsed = 0.0;
            live.battle = Some(crate::battle_view::LiveBattle::start(
                world,
                &mut live.battle_assets,
                stage,
            ));
        }
    }
    // The battle view covers the field from here; leave the field as it is
    // until the replay ends.
    if live.battle.is_some() {
        world.insert_resource(live);
        return;
    }
    let game = live.game.lock().expect("game lock");
    let background = game.background().map(str::to_owned);
    if background != live.background {
        live.background = background.clone();
        live.field = None;
        live.bundled = None;
        live.markers_at = None;
        live.motion = None;
        if let Some(name) = &background {
            match find_field(&live.streaming_assets, name) {
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
        if let Err(error) = show_field(world, field, &mut scene) {
            println!("!! cannot show {}: {error}", field.name);
        }
        live.scene = scene;
        live.field = Some(field.scene.clone());
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
        let (frame, placed) = world_frame(game.world(), game.ticks);
        if live.markers_at != Some(game.ticks) {
            let field_name = game.field_name().map(str::to_owned);
            for problem in live.models.prepare_field(field_name.as_deref()) {
                println!("!! field light: {problem}");
            }
            live.models.apply_light_changes(&game.world().light_changes);
            for problem in live.models.prepare(frame.poses.values()) {
                println!("!! actor models: {problem}");
            }
            spawn_frame_markers(world, &scene.camera, &frame, &placed, Some(&live.models));
            live.markers_at = Some(game.ticks);
        }
        crate::scene_adapter::apply_overlay_changes(
            world,
            scene,
            &game.world().overlays,
            player_position(&frame),
        );
        let at = live.view.view(
            &game.world().camera_log,
            &scene.camera,
            scene.camera_index,
            player_position(&frame),
            game.ticks,
        );
        place_camera(world, at);
        if let Some(motion) = &live.motion
            && !motion.is_idle()
        {
            motion.place_sprites(world, scene);
        }
        crate::dialog_view::sync_dialogs(world, &game, &scene.camera, &frame);
    }
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
}
