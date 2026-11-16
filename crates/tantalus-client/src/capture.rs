use crate::scene_adapter::{ClientError, ClientSceneAdapter, ResolvedAssets};
use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::camera::RenderTarget;
use bevy::image::TextureFormatPixelInfo;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, MapMode,
    PipelineCache, PollType, TexelCopyBufferInfo, TexelCopyBufferLayout, TextureFormat,
    TextureUsages,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderGraph, RenderQueue};
use bevy::render::{Extract, Render, RenderApp, RenderSystems};
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use crossbeam_channel::{Receiver, Sender};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tantalus_contracts::*;

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    pub width: u32,
    pub height: u32,
    pub pre_roll_frames: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            width: 320,
            height: 180,
            pre_roll_frames: 40,
        }
    }
}

#[derive(Resource)]
struct CaptureState {
    frames_remaining: u32,
    elapsed_updates: u32,
}

pub fn capture_snapshot(
    snapshot: &PresentationSnapshot,
    output: &Path,
    config: CaptureConfig,
) -> Result<(), ClientError> {
    capture_with(output, config, |adapter, target| {
        adapter.set_render_target(RenderTarget::Image(target.into()));
        adapter.apply(snapshot, &ResolvedAssets::default())
    })
}

pub fn capture_field(
    field: &tantalus_assets::pc::bundled_field::BundledField,
    output: &Path,
    config: CaptureConfig,
) -> Result<(), ClientError> {
    capture_field_with(field, output, config, false)
}

/// [`capture_field`], optionally with the projected walkmesh corners drawn
/// over the background ([`crate::projection::spawn_walkmesh_dots`]).
pub fn capture_field_with(
    field: &tantalus_assets::pc::bundled_field::BundledField,
    output: &Path,
    config: CaptureConfig,
    walkmesh: bool,
) -> Result<(), ClientError> {
    capture_with(output, config, |adapter, target| {
        let assets = crate::field_assets::resolve_field_assets(adapter, field)?;
        adapter.set_render_target(RenderTarget::Image(target.into()));
        adapter.apply_field(&field.scene, &assets)?;
        // The projection `view-field` uses, so a capture checks the window.
        crate::window::fit_field_frame(adapter.app_mut().world_mut());
        if walkmesh {
            crate::projection::spawn_walkmesh_dots(
                adapter.app_mut().world_mut(),
                &field.scene.camera,
                &field.scene.navigation,
            );
        }
        Ok(())
    })
}

fn capture_with(
    output: &Path,
    config: CaptureConfig,
    apply: impl FnOnce(&mut ClientSceneAdapter, Handle<Image>) -> Result<(), ClientError>,
) -> Result<(), ClientError> {
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb_u8(18, 24, 36)))
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        )
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .add_plugins(ImageCopyPlugin);

    let mut adapter = ClientSceneAdapter::with_app(app);
    let target = adapter
        .app_mut()
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(new_render_target(config.width, config.height));
    let cpu_image = adapter
        .app_mut()
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(new_render_target(config.width, config.height));
    let size = Extent3d {
        width: config.width,
        height: config.height,
        ..default()
    };
    adapter.app_mut().insert_resource(PendingImageCopier {
        source: target.clone(),
        size,
    });
    adapter.app_mut().add_systems(Startup, initialize_copier);
    adapter.app_mut().world_mut().spawn(ImageToSave(cpu_image));
    apply(&mut adapter, target)?;

    let result: Arc<Mutex<Option<Result<(), String>>>> = Arc::new(Mutex::new(None));
    adapter.app_mut().insert_resource(CaptureState {
        frames_remaining: config.pre_roll_frames,
        elapsed_updates: 0,
    });
    adapter
        .app_mut()
        .insert_resource(CaptureResult(result.clone()));
    adapter
        .app_mut()
        .insert_resource(CaptureOutput(output.to_path_buf()));
    adapter.app_mut().add_systems(PostUpdate, capture_frame);
    adapter.app_mut().run();

    let mut result = result
        .lock()
        .map_err(|_| ClientError::new("capture result lock was poisoned"))?;
    result
        .take()
        .unwrap_or_else(|| Err("capture ended before an image was produced".to_owned()))
        .map_err(ClientError::new)
}

#[derive(Resource)]
struct CaptureResult(Arc<Mutex<Option<Result<(), String>>>>);

#[derive(Resource)]
struct CaptureOutput(PathBuf);

fn new_render_target(width: u32, height: u32) -> Image {
    let mut image = Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    image
}

struct CapturedFrame {
    data: Vec<u8>,
    waiting_pipelines: usize,
}

#[derive(Resource, Deref)]
struct MainWorldReceiver(Receiver<CapturedFrame>);

#[derive(Resource, Deref)]
struct RenderWorldSender(Sender<CapturedFrame>);

struct ImageCopyPlugin;

impl Plugin for ImageCopyPlugin {
    fn build(&self, app: &mut App) {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let render_app = app
            .insert_resource(MainWorldReceiver(receiver))
            .sub_app_mut(RenderApp);
        render_app
            .insert_resource(RenderWorldSender(sender))
            .add_systems(ExtractSchedule, extract_image_copiers)
            .add_systems(
                Render,
                receive_image_from_buffer.after(RenderSystems::Render),
            )
            .add_systems(RenderGraph, copy_image_to_buffer);
    }
}

#[derive(Resource)]
struct PendingImageCopier {
    source: Handle<Image>,
    size: Extent3d,
}

fn initialize_copier(world: &mut World) {
    let (source, size) = {
        let pending = world.resource::<PendingImageCopier>();
        (pending.source.clone(), pending.size)
    };
    let device = world.resource::<RenderDevice>();
    world.spawn(ImageCopier::new(source, size, device));
}

#[derive(Clone, Component)]
struct ImageCopier {
    buffer: Buffer,
    source: Handle<Image>,
}

impl ImageCopier {
    fn new(source: Handle<Image>, size: Extent3d, device: &RenderDevice) -> Self {
        let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(size.width as usize * 4);
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("tantalus-image-copy"),
            size: (padded_bytes_per_row * size.height as usize) as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { buffer, source }
    }
}

#[derive(Clone, Default, Resource, Deref)]
struct ImageCopiers(Vec<ImageCopier>);

fn extract_image_copiers(mut commands: Commands, copiers: Extract<Query<&ImageCopier>>) {
    commands.insert_resource(ImageCopiers(copiers.iter().cloned().collect()));
}

fn copy_image_to_buffer(
    context: RenderContext,
    copiers: Res<ImageCopiers>,
    queue: Res<RenderQueue>,
    images: Res<RenderAssets<bevy::render::texture::GpuImage>>,
) {
    for copier in &copiers.0 {
        let Some(source) = images.get(&copier.source) else {
            continue;
        };
        let mut encoder = context
            .render_device()
            .create_command_encoder(&CommandEncoderDescriptor::default());
        let block_dimensions = source.texture_descriptor.format.block_dimensions();
        let block_size = source
            .texture_descriptor
            .format
            .block_copy_size(None)
            .unwrap();
        let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(
            (source.texture_descriptor.size.width as usize / block_dimensions.0 as usize)
                * block_size as usize,
        );
        encoder.copy_texture_to_buffer(
            source.texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &copier.buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row as u32),
                    rows_per_image: None,
                },
            },
            source.texture_descriptor.size,
        );
        queue.submit(std::iter::once(encoder.finish()));
    }
}

fn receive_image_from_buffer(
    copiers: Res<ImageCopiers>,
    device: Res<RenderDevice>,
    pipelines: Res<PipelineCache>,
    sender: Res<RenderWorldSender>,
) {
    let waiting_pipelines = pipelines.waiting_pipelines().count();
    for copier in &copiers.0 {
        let slice = copier.buffer.slice(..);
        let (send, receive) = crossbeam_channel::bounded(1);
        slice.map_async(MapMode::Read, move |result| {
            send.send(result).expect("image map channel");
        });
        device
            .poll(PollType::wait_indefinitely())
            .expect("image map device poll");
        receive
            .recv()
            .expect("image map result")
            .expect("image map");
        let _ = sender.send(CapturedFrame {
            data: slice.get_mapped_range().to_vec(),
            waiting_pipelines,
        });
        copier.buffer.unmap();
    }
}

#[derive(Component, Deref)]
struct ImageToSave(Handle<Image>);

fn capture_frame(
    images_to_save: Query<&ImageToSave>,
    receiver: Res<MainWorldReceiver>,
    mut images: ResMut<Assets<Image>>,
    mut state: ResMut<CaptureState>,
    result: Res<CaptureResult>,
    output: Res<CaptureOutput>,
    mut exit: MessageWriter<AppExit>,
) {
    state.elapsed_updates += 1;
    if state.elapsed_updates > 600 {
        *result.0.lock().expect("capture result mutex") = Some(Err(
            "capture timed out waiting for render pipelines".to_owned(),
        ));
        exit.write(AppExit::Success);
        return;
    }
    if state.frames_remaining > 0 {
        let mut received_ready_frame = false;
        while let Ok(frame) = receiver.try_recv() {
            received_ready_frame |= frame.waiting_pipelines == 0;
        }
        if received_ready_frame {
            state.frames_remaining -= 1;
        }
        return;
    }

    let mut latest = None;
    while let Ok(frame) = receiver.try_recv() {
        if frame.waiting_pipelines == 0 {
            latest = Some(frame.data);
        }
    }
    let Some(data) = latest else {
        return;
    };
    for image in images_to_save.iter() {
        let mut cpu_image = images.get_mut(image.id()).unwrap();
        let row_bytes =
            cpu_image.width() as usize * cpu_image.texture_descriptor.format.pixel_size().unwrap();
        let aligned = RenderDevice::align_copy_bytes_per_row(row_bytes);
        cpu_image.data = Some(if row_bytes == aligned {
            data.clone()
        } else {
            data.chunks(aligned)
                .take(cpu_image.height() as usize)
                .flat_map(|row| &row[..row_bytes.min(row.len())])
                .cloned()
                .collect()
        });
        let save_result = save_image(&cpu_image, &output.0);
        *result.0.lock().expect("capture result mutex") = Some(save_result);
    }
    exit.write(AppExit::Success);
}

fn save_image(image: &Image, output: &Path) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let image = image
        .clone()
        .try_into_dynamic()
        .map_err(|error| format!("could not convert captured image: {error}"))?
        .to_rgba8();
    image
        .save(output)
        .map_err(|error| format!("could not save {}: {error}", output.display()))
}

pub fn synthetic_snapshot(mode: ScenePresentationMode) -> PresentationSnapshot {
    let (layers, actors, camera) = match mode {
        ScenePresentationMode::Layered2d => (
            vec![
                FieldLayerPresentation {
                    id: ContentId::parse("fixture:layer/background").unwrap(),
                    position: Point3::default(),
                    size: Point3 {
                        x: 320_000,
                        y: 180_000,
                        z: 1,
                    },
                    color: [38, 72, 110, 255],
                    foreground: false,
                },
                FieldLayerPresentation {
                    id: ContentId::parse("fixture:layer/foreground").unwrap(),
                    position: Point3 {
                        x: -110_000,
                        y: -55_000,
                        z: 20_000,
                    },
                    size: Point3 {
                        x: 100_000,
                        y: 25_000,
                        z: 1,
                    },
                    color: [20, 28, 38, 255],
                    foreground: true,
                },
            ],
            vec![ActorPresentation {
                entity: EntityId::new(1),
                position: Point3 {
                    x: 0,
                    y: -15_000,
                    z: 10_000,
                },
                size: Point3 {
                    x: 24_000,
                    y: 32_000,
                    z: 1,
                },
                color: [220, 190, 110, 255],
                animation: Some(ContentId::parse("fixture:animation/idle").unwrap()),
            }],
            CameraSnapshot::default(),
        ),
        ScenePresentationMode::Hybrid | ScenePresentationMode::Full3d => (
            vec![FieldLayerPresentation {
                id: ContentId::parse("fixture:layer/ground").unwrap(),
                position: Point3::default(),
                size: Point3 {
                    x: 4_000,
                    y: 100,
                    z: 4_000,
                },
                color: [55, 90, 65, 255],
                foreground: false,
            }],
            vec![ActorPresentation {
                entity: EntityId::new(1),
                position: Point3 { x: 0, y: 500, z: 0 },
                size: Point3 {
                    x: 1_000,
                    y: 1_000,
                    z: 1_000,
                },
                color: [190, 150, 90, 255],
                animation: Some(ContentId::parse("fixture:animation/idle").unwrap()),
            }],
            CameraSnapshot {
                position: Point3 {
                    x: -2_500,
                    y: 4_500,
                    z: 9_000,
                },
                target: Point3::default(),
            },
        ),
    };

    PresentationSnapshot {
        tick: Tick::default(),
        effects: Vec::new(),
        actors,
        layers,
        camera,
        mode,
        audio_requests: Vec::new(),
        ui_requests: Vec::new(),
    }
}
