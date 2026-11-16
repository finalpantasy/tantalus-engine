//! Windowed field viewer: the scene `capture_field` renders offscreen, shown
//! in a desktop window instead. Same adapter, same sprites, same camera; the
//! only change is the camera's target (the primary window) and a projection
//! that keeps the field's 320x240 frame in view at any window size.

use crate::field_assets::resolve_field_assets;
use crate::scene_adapter::{ClientError, ClientSceneAdapter};
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
    adapter.app_mut().run();
    Ok(())
}

/// Gives every 2D camera a projection that always shows the whole field
/// frame, letterboxed to the window's aspect ratio.
pub fn fit_field_frame(world: &mut World) {
    let mut cameras = world.query_filtered::<Entity, With<Camera2d>>();
    let cameras: Vec<Entity> = cameras.iter(world).collect();
    for camera in cameras {
        world
            .entity_mut(camera)
            .insert(Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::AutoMin {
                    min_width: FIELD_FRAME.0,
                    min_height: FIELD_FRAME.1,
                },
                ..OrthographicProjection::default_2d()
            }));
    }
}
