//! The playable window: [`tantalus_tools::play::PlayGame`] driven by the
//! keyboard at the field's 30 ticks a second, over the current field's
//! background, with each actor drawn as a marker.
//!
//! Keys: arrows or WASD walk (and move a choice cursor), Enter, Space or
//! Z confirm, Backspace or X cancel, Q triangle, E square, Tab select,
//! P start. Dialog lines, battles and field changes print to the console
//! and the current line shows in the window title (the client has no font
//! yet).

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
    };
    (frame, placed)
}

/// Moves the 2D camera to follow the player (the frame's controlled
/// actor), kept inside the field camera's scroll range.
pub fn follow_player(world: &mut World, camera: &LegacyFieldCamera, frame: &TrackFrame) {
    let target = frame
        .player
        .and_then(|player| frame.actors.iter().find(|(id, _)| *id == player))
        .and_then(|(_, position)| crate::projection::project_field_point(camera, *position));
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
    camera: Option<LegacyFieldCamera>,
    scene: Vec<Entity>,
    elapsed: f64,
}

/// Opens the play window. `game` is already on its first field.
pub fn play(
    game: PlayGame,
    streaming_assets: std::path::PathBuf,
    scale: u32,
) -> Result<(), ClientError> {
    let scale = scale.clamp(1, 8);
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
        .insert_resource(Live {
            game: std::sync::Mutex::new(game),
            streaming_assets,
            background: None,
            camera: None,
            scene: Vec::new(),
            elapsed: 0.0,
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

fn live_tick(world: &mut World) {
    let delta = world.resource::<Time>().delta_secs_f64();
    let held = pad_bits(world.resource::<ButtonInput<KeyCode>>());
    let mut live = world.remove_resource::<Live>().expect("live state");
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
        }
        if steps == 4 {
            live.elapsed = 0.0;
        }
    }
    let game = live.game.lock().expect("game lock");
    let background = game.background().map(str::to_owned);
    if background != live.background {
        live.background = background.clone();
        live.camera = None;
        if let Some(name) = &background {
            match find_field(&live.streaming_assets, name) {
                Ok(field) => {
                    let mut scene = std::mem::take(&mut live.scene);
                    if let Err(error) = show_field(world, &field, &mut scene) {
                        println!("!! cannot show {name}: {error}");
                    }
                    live.scene = scene;
                    live.camera = Some(field.scene.camera.clone());
                }
                Err(error) => println!("!! no background {name}: {error}"),
            }
        }
    }
    if let Some(camera) = &live.camera {
        let (frame, placed) = world_frame(game.world(), game.ticks);
        spawn_frame_markers(world, camera, &frame, &placed);
        follow_player(world, camera, &frame);
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
