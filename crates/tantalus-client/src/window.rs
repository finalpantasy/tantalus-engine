//! Windowed field viewer: the scene `capture_field` renders offscreen, shown
//! in a desktop window instead. Same adapter, same sprites, same camera; the
//! only change is the camera's target (the primary window) and a projection
//! that keeps the field's 320x240 frame in view at any window size.

use crate::field_assets::resolve_field_assets;
use crate::scene_adapter::{ClientError, ClientSceneAdapter};
use crate::track::{ActorTrack, FRAMES_PER_SECOND, TrackFrame, spawn_frame_markers};
use bevy::camera::ScalingMode;
use bevy::prelude::*;
use bevy::window::WindowResolution;
use tantalus_assets::pc::bundled_field::BundledField;

/// The legacy field frame the capture path renders (`CaptureConfig` for
/// fields): the PSX-era 320x240 view.
pub const FIELD_FRAME: (f32, f32) = (320.0, 240.0);

/// Opens a window titled after the field and draws its background until the
/// window closes. `scale` multiplies the 320x240 frame for the initial
/// window size.
pub fn view_field(field: &BundledField, scale: u32) -> Result<(), ClientError> {
    let mut adapter = field_window(field, scale)?;
    adapter.app_mut().run();
    Ok(())
}

/// Plays an actor track over its field in a window: markers move at the
/// field's 30 ticks a second and the track loops when it ends.
pub fn view_track(field: &BundledField, track: ActorTrack, scale: u32) -> Result<(), ClientError> {
    let mut adapter = field_window(field, scale)?;
    let placed = track.placed_actors();
    adapter.app_mut().insert_resource(Playback {
        camera: field.scene.camera.clone(),
        frames: track.frames,
        placed,
        index: 0,
        elapsed: 0.0,
        started: false,
    });
    adapter.app_mut().add_systems(Update, advance_playback);
    adapter.app_mut().run();
    Ok(())
}

#[derive(Resource)]
struct Playback {
    camera: tantalus_contracts::LegacyFieldCamera,
    frames: Vec<TrackFrame>,
    placed: std::collections::BTreeSet<u64>,
    index: usize,
    elapsed: f64,
    started: bool,
}

fn advance_playback(world: &mut World) {
    let delta = world.resource::<Time>().delta_secs_f64();
    let (frame, camera, placed) = {
        let mut playback = world.resource_mut::<Playback>();
        if playback.frames.is_empty() {
            return;
        }
        let step = 1.0 / FRAMES_PER_SECOND;
        playback.elapsed += delta;
        if playback.started && playback.elapsed < step {
            return;
        }
        if playback.started {
            while playback.elapsed >= step {
                playback.elapsed -= step;
                playback.index = (playback.index + 1) % playback.frames.len();
            }
        }
        playback.started = true;
        (
            playback.frames[playback.index].clone(),
            playback.camera.clone(),
            playback.placed.clone(),
        )
    };
    spawn_frame_markers(world, &camera, &frame, &placed, None);
}

fn field_window(field: &BundledField, scale: u32) -> Result<ClientSceneAdapter, ClientError> {
    let scale = scale.clamp(1, 8);
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb_u8(18, 24, 36)))
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: format!("Tantalus Engine: {}", field.name),
                        resolution: WindowResolution::new(
                            FIELD_FRAME.0 as u32 * scale,
                            FIELD_FRAME.1 as u32 * scale,
                        ),
                        ..default()
                    }),
                    ..default()
                }),
        );
    let mut adapter = ClientSceneAdapter::with_app(app);
    let assets = resolve_field_assets(&mut adapter, field)?;
    adapter.apply_field(&field.scene, &assets)?;
    fit_field_frame(adapter.app_mut().world_mut());
    Ok(adapter)
}

/// Loads `name` from whichever field bundle (`p0data1N1.bin`) in the
/// `StreamingAssets` directory holds it.
pub fn find_field(
    streaming_assets: &std::path::Path,
    name: &str,
) -> Result<BundledField, ClientError> {
    let mut last = None;
    for bank in 10..=19 {
        let bundle = streaming_assets.join(format!("p0data{bank}1.bin"));
        if !bundle.exists() {
            continue;
        }
        match BundledField::load(&bundle, name) {
            Ok(field) => return Ok(field),
            Err(error) => last = Some(error.to_string()),
        }
    }
    Err(ClientError::new(format!(
        "no field bundle in {} holds `{name}`{}",
        streaming_assets.display(),
        last.map(|error| format!(" (last error: {error})"))
            .unwrap_or_default()
    )))
}

/// Rows of the field the game shows (`FieldMap.PsxFieldHeightNative`): the
/// field camera's viewport (`PSXCameraAspect.LateUpdate`) is this many rows
/// of the 240-row frame, centered, so nothing of the field (background,
/// models, effects) draws in the 8 rows above and below it.
pub const FIELD_ROWS: f32 = 224.0;

/// Gives the field's 2D camera a projection that always shows the whole
/// field view. With a known target size ([`crate::widescreen::set_target`])
/// the view is the widescreen one: `widescreen::field_width()` art pixels
/// over [`FIELD_ROWS`] rows, in the centered viewport of the 240-row frame's
/// aspect less its top and bottom 8 rows (pillarboxed where the art is
/// narrower, letterboxed to a fixed `--aspect`). Without one, the whole
/// 320x240 frame fits the window.
pub fn fit_field_frame(world: &mut World) {
    use bevy::camera::Viewport;
    use bevy::camera::visibility::RenderLayers;
    let mut cameras = world.query_filtered::<Entity, (With<Camera2d>, Without<RenderLayers>)>();
    let cameras: Vec<Entity> = cameras.iter(world).collect();
    let (view_width, view_height) = (crate::widescreen::field_width(), FIELD_FRAME.1);
    let viewport = crate::widescreen::target().map(|(width, height)| {
        let (sx, sy, sw, sh) = crate::widescreen::screen_rect(width, height);
        let (x, y, w, h) = crate::widescreen::fit_rect(sw, sh, view_width / view_height);
        let (top, rows) = field_rows(h);
        Viewport {
            physical_position: UVec2::new(sx + x, sy + y + top),
            physical_size: UVec2::new(w.max(1), rows.max(1)),
            ..default()
        }
    });
    for camera in cameras {
        let scaling_mode = if let Some(viewport) = &viewport {
            // The art pixels the cut viewport holds, so the scale stays the 240-row frame's.
            let rows = viewport.physical_size.y as f32;
            let frame_rows = rows * view_height / FIELD_ROWS;
            ScalingMode::Fixed {
                width: view_width,
                height: view_height * rows / frame_rows.max(1.0),
            }
        } else {
            ScalingMode::AutoMin {
                min_width: FIELD_FRAME.0,
                min_height: FIELD_FRAME.1,
            }
        };
        let mut entity = world.entity_mut(camera);
        entity.insert(Projection::Orthographic(OrthographicProjection {
            scaling_mode,
            ..OrthographicProjection::default_2d()
        }));
        if let Some(mut camera) = entity.get_mut::<Camera>() {
            camera.viewport = viewport.clone();
        }
    }
}

/// The viewport rows of the field in a 240-row frame `height` pixels tall: the first row and
/// the count ([`FIELD_ROWS`] of the 240, centered).
pub fn field_rows(height: u32) -> (u32, u32) {
    let cut = ((height as f32 * (FIELD_FRAME.1 - FIELD_ROWS) / FIELD_FRAME.1) / 2.0).round() as u32;
    (cut, height.saturating_sub(2 * cut))
}

/// Fits the camera showing a battle or world frame of `frame` pixels to the
/// target: the frame's own aspect, centered (letterboxed to a fixed
/// `--aspect`, pillarboxed if the window changed shape since the frame was
/// made). Call each time the frame is shown.
pub fn fit_frame_camera(world: &mut World, camera: Entity, frame: (u32, u32)) {
    use bevy::camera::Viewport;
    let aspect = frame.0 as f32 / frame.1.max(1) as f32;
    let (view_width, view_height) = (FIELD_FRAME.1 * aspect, FIELD_FRAME.1);
    let viewport = crate::widescreen::target().map(|(width, height)| {
        let (sx, sy, sw, sh) = crate::widescreen::screen_rect(width, height);
        let (x, y, w, h) = crate::widescreen::fit_rect(sw, sh, aspect);
        Viewport {
            physical_position: UVec2::new(sx + x, sy + y),
            physical_size: UVec2::new(w.max(1), h.max(1)),
            ..default()
        }
    });
    let Ok(mut entity) = world.get_entity_mut(camera) else {
        return;
    };
    entity.insert(Projection::Orthographic(OrthographicProjection {
        scaling_mode: ScalingMode::Fixed {
            width: view_width,
            height: view_height,
        },
        ..OrthographicProjection::default_2d()
    }));
    if let Some(mut camera) = entity.get_mut::<Camera>() {
        camera.viewport = viewport;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_field_viewport_is_224_of_240_rows() {
        assert_eq!(super::field_rows(240), (8, 224));
        assert_eq!(super::field_rows(1080), (36, 1008));
        assert_eq!(super::field_rows(360), (12, 336));
    }
}
