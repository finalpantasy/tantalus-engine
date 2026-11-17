//! The battle and world-map frame on the GPU. A [`Frame`] made with
//! [`Frame::for_view`] while this backend is on records what the CPU
//! rasterizer would draw (clipped, projected triangles, rectangles, HUD
//! images and the world's fog pass) into a [`Recording`], batched by
//! texture and draw state but kept in draw order. [`GpuRaster`] replays it
//! with `shaders/frame_gpu.wgsl` into a plain RGBA8 view of the frame
//! image, so the Screen, Multiply and alpha blends work on the same
//! sRGB-encoded bytes the CPU blends, and the depth buffer holds
//! 1 / view distance, the value the CPU compares.
//!
//! The play window replays through [`FrameGpuPlugin`] in the render world;
//! captures that save a frame without a window ([`enable_headless`]) use a
//! device of their own and read the pixels back. `TANTALUS_RASTER_CPU=1`
//! keeps the CPU rasterizer everywhere.
//!
//! [`Frame`]: crate::battle_raster::Frame
//! [`Frame::for_view`]: crate::battle_raster::Frame::for_view

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use wgpu::util::DeviceExt;

/// One vertex: clip position (x, y, z, w), uv, color (r, g, b, a), mode
/// (1 for a box-filtered image) and source texels per frame pixel.
pub type Vertex = [f32; 12];

const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Renders a GPU texture may go unused before it is dropped.
const KEEP_RENDERS: u64 = 600;

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether new view frames record for the GPU.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// `TANTALUS_RASTER_CPU=1`: keep the CPU rasterizer.
pub fn cpu_forced() -> bool {
    std::env::var("TANTALUS_RASTER_CPU").is_ok_and(|value| !value.is_empty() && value != "0")
}

/// How a draw writes its pixels (`battle_raster::Blend`, plus the
/// rectangles' and images' straight alpha).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendKind {
    /// Cut out under alpha 0.5, then replace.
    Replace,
    /// `One OneMinusSrcColor`.
    Screen,
    /// `dst * src`.
    Multiply,
    /// Straight alpha.
    Over,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineKey {
    pub blend: BlendKind,
    pub depth_test: bool,
    pub depth_write: bool,
    /// Drop triangles that wind counter-clockwise on screen (only static
    /// meshes; recorded triangles are culled as they are recorded).
    pub cull_back: bool,
}

/// How a draw reads its texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sampling {
    Clamp,
    Repeat,
    /// Box-filtered (the HUD's scaled images); reads texels directly.
    Box,
}

/// The world's sky and distance fog (`world_weather::Weather::apply`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fog {
    /// Overhead sky color; it turns to `fog` at the horizon row.
    pub sky: [u8; 3],
    pub fog: [u8; 3],
    /// Frame row of the horizon.
    pub horizon: f32,
    /// View distances the fog starts and is full at.
    pub start: f32,
    pub end: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Draw {
        key: PipelineKey,
        texture: u32,
        sampling: Sampling,
        first: u32,
        count: u32,
    },
    /// A static mesh ([`MeshData`]) through a transform.
    Mesh {
        key: PipelineKey,
        texture: u32,
        sampling: Sampling,
        mesh: u32,
        matrix: u32,
    },
    Fog(Fog),
}

/// Vertices that do not change from frame to frame (a world block's
/// submesh): positions in the mesh's own space, kept on the GPU by `key`.
#[derive(Debug, Clone)]
pub struct MeshData {
    pub key: u64,
    pub vertices: Arc<[Vertex]>,
}

impl MeshData {
    /// `vertices` with a key no other mesh has.
    pub fn new(vertices: Vec<Vertex>) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            key: NEXT.fetch_add(1, Ordering::Relaxed),
            vertices: Arc::from(vertices),
        }
    }
}

/// A static-mesh vertex: position, uv (`v = 0` the last texel row, as
/// [`crate::battle_raster::Texture`] has it), color.
pub fn mesh_vertex(position: [f32; 3], uv: [f32; 2], color: [f32; 4]) -> Vertex {
    [
        position[0],
        position[1],
        position[2],
        1.0,
        uv[0],
        1.0 - uv[1],
        color[0],
        color[1],
        color[2],
        color[3],
        0.0,
        0.0,
    ]
}

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// Texels a recording draws with, shared with the cache that copied them.
#[derive(Debug, Clone)]
pub struct TextureData {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

/// What one frame draws, in order.
#[derive(Debug, Clone)]
pub struct Recording {
    pub width: u32,
    pub height: u32,
    pub clear: [u8; 4],
    pub vertices: Vec<Vertex>,
    pub commands: Vec<Command>,
    pub textures: Vec<TextureData>,
    pub meshes: Vec<MeshData>,
    /// Column-major transforms to clip space; the first is the identity
    /// (recorded triangles are in clip space already).
    pub matrices: Vec<[f32; 16]>,
    /// The last texture looked up: (pointer, length, width, height) and its
    /// index, since triangles come in runs over one texture.
    memo: Option<(Identity, u32)>,
}

impl PartialEq for Recording {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.clear == other.clear
            && self.vertices == other.vertices
            && self.commands == other.commands
            && self.matrices == other.matrices
            && self.meshes.len() == other.meshes.len()
            && self.textures.len() == other.textures.len()
            && self
                .textures
                .iter()
                .zip(&other.textures)
                .all(|(a, b)| a.key == b.key)
    }
}

const WHITE_KEY: u64 = 0x5748_4954_455f_3178;

impl Recording {
    pub fn new(width: u32, height: u32, clear: [u8; 4]) -> Self {
        host().lock().expect("texture cache").generation += 1;
        Self {
            width,
            height,
            clear,
            vertices: Vec::new(),
            commands: Vec::new(),
            textures: Vec::new(),
            meshes: Vec::new(),
            matrices: vec![IDENTITY],
            memo: None,
        }
    }

    /// Draws `mesh` through `matrix` (column-major, to clip space).
    pub fn mesh(
        &mut self,
        key: PipelineKey,
        texture: u32,
        sampling: Sampling,
        mesh: &MeshData,
        matrix: [f32; 16],
    ) {
        if mesh.vertices.is_empty() {
            return;
        }
        let mesh_index = match self.meshes.iter().rposition(|m| m.key == mesh.key) {
            Some(index) => index,
            None => {
                self.meshes.push(mesh.clone());
                self.meshes.len() - 1
            }
        };
        let matrix_index = match self.matrices.last() {
            Some(last) if *last == matrix => self.matrices.len() - 1,
            _ => {
                self.matrices.push(matrix);
                self.matrices.len() - 1
            }
        };
        self.commands.push(Command::Mesh {
            key,
            texture,
            sampling,
            mesh: mesh_index as u32,
            matrix: matrix_index as u32,
        });
    }

    fn add_texture(&mut self, data: TextureData) -> u32 {
        if let Some(index) = self.textures.iter().position(|t| t.key == data.key) {
            return index as u32;
        }
        self.textures.push(data);
        (self.textures.len() - 1) as u32
    }

    /// The index of a texture a triangle draws with (`None`: plain white).
    /// Textures are told apart by where their texels live, their size and a
    /// sample of them, as the field actors' atlases are.
    pub fn texture(&mut self, rgba: Option<(&[u8], usize, usize)>) -> u32 {
        let Some((rgba, width, height)) = rgba else {
            return self.add_texture(white());
        };
        let identity = (rgba.as_ptr() as usize, rgba.len(), width, height);
        if let Some((last, index)) = self.memo
            && last == identity
        {
            return index;
        }
        let sample = sample_hash(rgba);
        let data = {
            let mut cache = host().lock().expect("texture cache");
            let generation = cache.generation;
            let entry = cache.textures.entry((identity, sample)).or_insert_with(|| {
                (
                    TextureData {
                        key: mix(mix(mix(identity.0 as u64, identity.1 as u64), sample), {
                            (width as u64) << 32 | height as u64
                        }),
                        width: width as u32,
                        height: height as u32,
                        rgba: Arc::from(&rgba[..width * height * 4]),
                    },
                    generation,
                )
            });
            entry.1 = generation;
            entry.0.clone()
        };
        let index = self.add_texture(data);
        self.memo = Some((identity, index));
        index
    }

    /// The index of a short-lived image (the HUD's text and windows), told
    /// apart by its whole contents.
    pub fn image_texture(&mut self, rgba: &[u8], width: usize, height: usize) -> u32 {
        let key = mix(content_hash(rgba), (width as u64) << 32 | height as u64);
        if let Some(index) = self.textures.iter().position(|t| t.key == key) {
            return index as u32;
        }
        let data = {
            let mut cache = host().lock().expect("texture cache");
            let generation = cache.generation;
            let entry = cache.images.entry(key).or_insert_with(|| {
                (
                    TextureData {
                        key,
                        width: width as u32,
                        height: height as u32,
                        rgba: Arc::from(&rgba[..width * height * 4]),
                    },
                    generation,
                )
            });
            entry.1 = generation;
            entry.0.clone()
        };
        self.add_texture(data)
    }

    /// Appends vertices as one draw, joined to the last draw when it has the
    /// same state.
    pub fn push(
        &mut self,
        key: PipelineKey,
        texture: u32,
        sampling: Sampling,
        vertices: &[Vertex],
    ) {
        let start = self.vertices.len() as u32;
        self.vertices.extend_from_slice(vertices);
        if let Some(Command::Draw {
            key: last_key,
            texture: last_texture,
            sampling: last_sampling,
            first,
            count,
        }) = self.commands.last_mut()
            && *last_key == key
            && *last_texture == texture
            && *last_sampling == sampling
            && *first + *count == start
        {
            *count += vertices.len() as u32;
            return;
        }
        self.commands.push(Command::Draw {
            key,
            texture,
            sampling,
            first: start,
            count: vertices.len() as u32,
        });
    }

    /// A screen point (frame pixels) as a flat vertex.
    pub fn flat(&self, x: f32, y: f32, uv: [f32; 2], color: [f32; 4], mode: [f32; 2]) -> Vertex {
        let (nx, ny) = self.ndc(x, y);
        [
            nx, ny, 0.0, 1.0, uv[0], uv[1], color[0], color[1], color[2], color[3], mode[0],
            mode[1],
        ]
    }

    pub fn ndc(&self, x: f32, y: f32) -> (f32, f32) {
        (
            x / self.width as f32 * 2.0 - 1.0,
            1.0 - y / self.height as f32 * 2.0,
        )
    }

    /// `Frame::rect`: a filled rectangle blended at `alpha`.
    pub fn rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: [u8; 3], alpha: f32) {
        let alpha = alpha.clamp(0.0, 1.0);
        if width <= 0 || height <= 0 || alpha <= 0.0 {
            return;
        }
        let texture = self.add_texture(white());
        let color = [
            f32::from(color[0]) / 255.0,
            f32::from(color[1]) / 255.0,
            f32::from(color[2]) / 255.0,
            alpha,
        ];
        let (x0, y0) = (x as f32, y as f32);
        let (x1, y1) = ((x + width) as f32, (y + height) as f32);
        let corner = |x: f32, y: f32| self.flat(x, y, [0.5, 0.5], color, [0.0, 0.0]);
        let quad = quad([
            corner(x0, y0),
            corner(x1, y0),
            corner(x1, y1),
            corner(x0, y1),
        ]);
        self.push(OVER, texture, Sampling::Clamp, &quad);
    }

    /// `Frame::blit`: an opaque image copied pixel for pixel.
    pub fn blit(&mut self, x: i32, y: i32, width: i32, height: i32, rgba: &[u8]) {
        let texture = self.image_texture(rgba, width as usize, height as usize);
        let (x0, y0) = (x as f32, y as f32);
        let (x1, y1) = ((x + width) as f32, (y + height) as f32);
        let corner = |px: f32, py: f32, uv: [f32; 2]| self.flat(px, py, uv, [1.0; 4], [0.0, 0.0]);
        let quad = quad([
            corner(x0, y0, [0.0, 0.0]),
            corner(x1, y0, [1.0, 0.0]),
            corner(x1, y1, [1.0, 1.0]),
            corner(x0, y1, [0.0, 1.0]),
        ]);
        self.push(OVER, texture, Sampling::Clamp, &quad);
    }

    /// `battle_hud::blit_scaled`: `rgba` (straight alpha) with its top left
    /// at `(x, y)` UI pixels, scaled by `scale` with a box filter.
    #[allow(clippy::too_many_arguments)]
    pub fn image(&mut self, x: f32, y: f32, scale: f32, rgba: &[u8], width: usize, height: usize) {
        if width == 0 || height == 0 || rgba.len() < width * height * 4 {
            return;
        }
        let x0 = (x * scale).floor();
        let y0 = (y * scale).floor();
        let x1 = ((x + width as f32) * scale).ceil();
        let y1 = ((y + height as f32) * scale).ceil();
        let inverse = 1.0 / scale;
        let texture = self.image_texture(rgba, width, height);
        // The texel under a frame point, as a fraction of the image.
        let uv = |px: f32, py: f32| {
            [
                (px * inverse - x) / width as f32,
                (py * inverse - y) / height as f32,
            ]
        };
        let corner = |px: f32, py: f32| self.flat(px, py, uv(px, py), [1.0; 4], [1.0, inverse]);
        let quad = quad([
            corner(x0, y0),
            corner(x1, y0),
            corner(x1, y1),
            corner(x0, y1),
        ]);
        self.push(OVER, texture, Sampling::Box, &quad);
    }

    /// The world's sky and fog over what is drawn so far.
    pub fn fog(&mut self, fog: Fog) {
        self.commands.push(Command::Fog(fog));
    }
}

const OVER: PipelineKey = PipelineKey {
    blend: BlendKind::Over,
    depth_test: false,
    depth_write: false,
    cull_back: false,
};

fn quad(corners: [Vertex; 4]) -> [Vertex; 6] {
    [
        corners[0], corners[1], corners[2], corners[0], corners[2], corners[3],
    ]
}

fn white() -> TextureData {
    static WHITE: std::sync::OnceLock<Arc<[u8]>> = std::sync::OnceLock::new();
    TextureData {
        key: WHITE_KEY,
        width: 1,
        height: 1,
        rgba: WHITE
            .get_or_init(|| Arc::from(&[255u8, 255, 255, 255][..]))
            .clone(),
    }
}

fn mix(hash: u64, value: u64) -> u64 {
    (hash ^ value)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .rotate_left(29)
        ^ 0xA076_1D64_78BD_642F
}

/// A hash of 256 words spread over `rgba`.
fn sample_hash(rgba: &[u8]) -> u64 {
    let words = rgba.len() / 4;
    if words == 0 {
        return 0;
    }
    let step = (words / 256).max(1);
    (0..words)
        .step_by(step)
        .fold(rgba.len() as u64, |hash, word| {
            let at = word * 4;
            mix(
                hash,
                u64::from(u32::from_le_bytes([
                    rgba[at],
                    rgba[at + 1],
                    rgba[at + 2],
                    rgba[at + 3],
                ])),
            )
        })
}

/// A hash of every byte of `rgba`.
fn content_hash(rgba: &[u8]) -> u64 {
    let mut chunks = rgba.chunks_exact(8);
    let mut hash = rgba.len() as u64;
    for chunk in &mut chunks {
        hash = mix(hash, u64::from_le_bytes(chunk.try_into().expect("8 bytes")));
    }
    for &byte in chunks.remainder() {
        hash = mix(hash, u64::from(byte));
    }
    hash
}

/// Where a texture's texels live and its size: pointer, length, width,
/// height.
type Identity = (usize, usize, usize, usize);

/// Copies of the textures recordings drew with lately, so a recording can
/// hand its texels to the render thread without copying them every frame.
#[derive(Default)]
struct HostTextures {
    /// By (pointer, length, width, height) and a sample hash; the last
    /// generation that used each.
    textures: HashMap<(Identity, u64), (TextureData, u64)>,
    /// Images by their content hash.
    images: HashMap<u64, (TextureData, u64)>,
    /// Counts recordings made.
    generation: u64,
}

fn host() -> &'static Mutex<HostTextures> {
    static HOST: std::sync::OnceLock<Mutex<HostTextures>> = std::sync::OnceLock::new();
    HOST.get_or_init(|| Mutex::new(HostTextures::default()))
}

/// Drops cached copies no recording used for a while.
fn trim_host() {
    let mut cache = host().lock().expect("texture cache");
    let now = cache.generation;
    // Several recordings are made per shown frame (the base, the frame,
    // the unit-height probes); keep anything used in the last few hundred.
    let keep = |used: u64| now.saturating_sub(used) < 2000;
    if cache.textures.len() > 64 {
        cache.textures.retain(|_, (_, used)| keep(*used));
    }
    if cache.images.len() > 64 {
        cache.images.retain(|_, (_, used)| keep(*used));
    }
}

struct GpuTexture {
    view: wgpu::TextureView,
    /// Bind groups with the clamping and the repeating sampler.
    groups: [Option<wgpu::BindGroup>; 2],
    used: u64,
}

/// The replayer: pipelines per draw state, the textures recordings drew
/// with, and the depth buffer.
pub struct GpuRaster {
    shader: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    fog_layout: wgpu::BindGroupLayout,
    matrix_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    fog_pipeline: wgpu::RenderPipeline,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    samplers: [wgpu::Sampler; 2],
    textures: HashMap<u64, GpuTexture>,
    /// Static mesh vertex buffers and the render that last drew each.
    meshes: HashMap<u64, (wgpu::Buffer, u64)>,
    depth: Option<(u32, u32, wgpu::TextureView)>,
    renders: u64,
}

impl GpuRaster {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tantalus-frame-gpu"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/frame_gpu.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tantalus-frame-gpu-texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let fog_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tantalus-frame-gpu-fog"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let matrix_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tantalus-frame-gpu-matrices"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tantalus-frame-gpu"),
            bind_group_layouts: &[Some(&layout), Some(&matrix_layout)],
            immediate_size: 0,
        });
        let fog_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tantalus-frame-gpu-fog"),
            bind_group_layouts: &[Some(&fog_layout)],
            immediate_size: 0,
        });
        let fog_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tantalus-frame-gpu-fog"),
            layout: Some(&fog_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("fog_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fog_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TARGET_FORMAT,
                    blend: Some(blend_state(BlendKind::Over)),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = |mode: wgpu::AddressMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("tantalus-frame-gpu"),
                address_mode_u: mode,
                address_mode_v: mode,
                address_mode_w: mode,
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            })
        };
        let samplers = [
            sampler(wgpu::AddressMode::ClampToEdge),
            sampler(wgpu::AddressMode::Repeat),
        ];
        Self {
            shader,
            layout,
            fog_layout,
            matrix_layout,
            pipeline_layout,
            fog_pipeline,
            pipelines: HashMap::new(),
            samplers,
            textures: HashMap::new(),
            meshes: HashMap::new(),
            depth: None,
            renders: 0,
        }
    }

    fn pipeline(&mut self, device: &wgpu::Device, key: PipelineKey) {
        if self.pipelines.contains_key(&key) {
            return;
        }
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x4,
            1 => Float32x2,
            2 => Float32x4,
            3 => Float32x2
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tantalus-frame-gpu"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                }],
            },
            // Clockwise on screen is clockwise in clip space too (only y
            // turns over, and the window's rows run down).
            primitive: wgpu::PrimitiveState {
                front_face: wgpu::FrontFace::Cw,
                cull_mode: key.cull_back.then_some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(key.depth_write),
                // Depth holds 1 / distance: nearer is greater.
                depth_compare: Some(if key.depth_test {
                    wgpu::CompareFunction::Greater
                } else {
                    wgpu::CompareFunction::Always
                }),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some(if key.blend == BlendKind::Replace {
                    "cutout"
                } else {
                    "plain"
                }),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TARGET_FORMAT,
                    blend: (key.blend != BlendKind::Replace).then(|| blend_state(key.blend)),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key, pipeline);
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &TextureData) {
        if let Some(texture) = self.textures.get_mut(&data.key) {
            texture.used = self.renders;
            return;
        }
        let size = wgpu::Extent3d {
            width: data.width.max(1),
            height: data.height.max(1),
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("tantalus-frame-gpu-texels"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &data.rgba,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.textures.insert(
            data.key,
            GpuTexture {
                view,
                groups: [None, None],
                used: self.renders,
            },
        );
    }

    fn bind_group(&mut self, device: &wgpu::Device, key: u64, sampling: Sampling) {
        let slot = usize::from(sampling == Sampling::Repeat);
        let Some(texture) = self.textures.get_mut(&key) else {
            return;
        };
        if texture.groups[slot].is_none() {
            texture.groups[slot] = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tantalus-frame-gpu-texture"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.samplers[slot]),
                    },
                ],
            }));
        }
    }

    fn depth_view(&mut self, device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
        if let Some((w, h, view)) = &self.depth
            && (*w, *h) == (width, height)
        {
            return view.clone();
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tantalus-frame-gpu-depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.depth = Some((width, height, view.clone()));
        view
    }

    /// Draws `recording` into `target`, a [`TARGET_FORMAT`] view the
    /// recording's size.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        recording: &Recording,
        target: &wgpu::TextureView,
    ) {
        self.renders += 1;
        for data in &recording.textures {
            self.upload(device, queue, data);
        }
        for command in &recording.commands {
            let (Command::Draw {
                key,
                texture,
                sampling,
                ..
            }
            | Command::Mesh {
                key,
                texture,
                sampling,
                ..
            }) = command
            else {
                continue;
            };
            self.pipeline(device, *key);
            let data_key = recording.textures[*texture as usize].key;
            self.bind_group(device, data_key, *sampling);
        }
        let renders = self.renders;
        for mesh in &recording.meshes {
            self.meshes
                .entry(mesh.key)
                .and_modify(|entry| entry.1 = renders)
                .or_insert_with(|| {
                    (
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("tantalus-frame-gpu-mesh"),
                            contents: bytemuck::cast_slice(&mesh.vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        }),
                        renders,
                    )
                });
        }
        let matrices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tantalus-frame-gpu-matrices"),
            contents: bytemuck::cast_slice(&recording.matrices),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let matrix_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tantalus-frame-gpu-matrices"),
            layout: &self.matrix_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: matrices.as_entire_binding(),
            }],
        });
        let depth = self.depth_view(device, recording.width, recording.height);
        let vertices = (!recording.vertices.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tantalus-frame-gpu-vertices"),
                contents: bytemuck::cast_slice(&recording.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let clear = recording.clear.map(|c| f64::from(c) / 255.0);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("tantalus-frame-gpu"),
        });
        let mut start = 0;
        let mut first_pass = true;
        loop {
            let end = recording.commands[start..]
                .iter()
                .position(|command| matches!(command, Command::Fog(_)))
                .map_or(recording.commands.len(), |at| start + at);
            {
                let load = |clear| {
                    if first_pass {
                        wgpu::LoadOp::Clear(clear)
                    } else {
                        wgpu::LoadOp::Load
                    }
                };
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("tantalus-frame-gpu"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: load(wgpu::Color {
                                r: clear[0],
                                g: clear[1],
                                b: clear[2],
                                a: clear[3],
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: if first_pass {
                                wgpu::LoadOp::Clear(0.0)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(1, &matrix_group, &[]);
                // The bound vertex buffer: the recording's (None) or a mesh's.
                let mut bound: Option<Option<u64>> = None;
                for command in &recording.commands[start..end] {
                    let (key, texture, sampling, source, range, instance) = match command {
                        Command::Draw {
                            key,
                            texture,
                            sampling,
                            first,
                            count,
                        } => (key, texture, sampling, None, *first..*first + *count, 0),
                        Command::Mesh {
                            key,
                            texture,
                            sampling,
                            mesh,
                            matrix,
                        } => {
                            let mesh = &recording.meshes[*mesh as usize];
                            (
                                key,
                                texture,
                                sampling,
                                Some(mesh.key),
                                0..mesh.vertices.len() as u32,
                                *matrix,
                            )
                        }
                        Command::Fog(_) => continue,
                    };
                    let data_key = recording.textures[*texture as usize].key;
                    let slot = usize::from(*sampling == Sampling::Repeat);
                    let Some(group) = self
                        .textures
                        .get(&data_key)
                        .and_then(|texture| texture.groups[slot].as_ref())
                    else {
                        continue;
                    };
                    if bound != Some(source) {
                        let buffer = match source {
                            None => vertices.as_ref(),
                            Some(mesh) => self.meshes.get(&mesh).map(|(buffer, _)| buffer),
                        };
                        let Some(buffer) = buffer else {
                            continue;
                        };
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        bound = Some(source);
                    }
                    pass.set_pipeline(&self.pipelines[key]);
                    pass.set_bind_group(0, group, &[]);
                    pass.draw(range, instance..instance + 1);
                }
            }
            first_pass = false;
            let Some(Command::Fog(fog)) = recording.commands.get(end) else {
                break;
            };
            let color = |c: [u8; 3]| c.map(|v| f32::from(v) / 255.0);
            let (sky, tint) = (color(fog.sky), color(fog.fog));
            let values: [f32; 12] = [
                sky[0],
                sky[1],
                sky[2],
                1.0,
                tint[0],
                tint[1],
                tint[2],
                1.0,
                fog.horizon,
                fog.start,
                fog.end,
                0.0,
            ];
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tantalus-frame-gpu-fog"),
                contents: bytemuck::cast_slice(&values),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tantalus-frame-gpu-fog"),
                layout: &self.fog_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("tantalus-frame-gpu-fog"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.fog_pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            }
            start = end + 1;
        }
        queue.submit(std::iter::once(encoder.finish()));
        let now = self.renders;
        self.textures
            .retain(|_, texture| now.saturating_sub(texture.used) < KEEP_RENDERS);
        self.meshes
            .retain(|_, (_, used)| now.saturating_sub(*used) < KEEP_RENDERS);
        trim_host();
    }
}

fn blend_state(blend: BlendKind) -> wgpu::BlendState {
    use wgpu::{BlendComponent, BlendFactor, BlendOperation};
    let keep_alpha = BlendComponent {
        src_factor: BlendFactor::Zero,
        dst_factor: BlendFactor::One,
        operation: BlendOperation::Add,
    };
    let color = match blend {
        BlendKind::Replace => BlendComponent::REPLACE,
        BlendKind::Screen => BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::OneMinusSrc,
            operation: BlendOperation::Add,
        },
        BlendKind::Multiply => BlendComponent {
            src_factor: BlendFactor::Zero,
            dst_factor: BlendFactor::Src,
            operation: BlendOperation::Add,
        },
        BlendKind::Over => BlendComponent {
            src_factor: BlendFactor::SrcAlpha,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        },
    };
    wgpu::BlendState {
        color,
        alpha: keep_alpha,
    }
}

/// A device of its own for frames saved without a window.
struct Headless {
    device: wgpu::Device,
    queue: wgpu::Queue,
    raster: GpuRaster,
}

static HEADLESS: Mutex<Option<Headless>> = Mutex::new(None);

/// Turns the GPU backend on for frames saved to files (captures), with a
/// device of its own. False, and the CPU rasterizer stays, when
/// `TANTALUS_RASTER_CPU` is set or no adapter answers.
pub fn enable_headless() -> bool {
    if cpu_forced() {
        return false;
    }
    let mut headless = HEADLESS.lock().expect("headless lock");
    if headless.is_none() {
        match open_device() {
            Ok((device, queue)) => {
                let raster = GpuRaster::new(&device);
                *headless = Some(Headless {
                    device,
                    queue,
                    raster,
                });
            }
            Err(error) => {
                eprintln!("!! GPU frames off, drawing on the CPU: {error}");
                return false;
            }
        }
    }
    ACTIVE.store(true, Ordering::Relaxed);
    true
}

fn open_device() -> Result<(wgpu::Device, wgpu::Queue), String> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = bevy::tasks::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .map_err(|error| error.to_string())?;
    bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("tantalus-frame-gpu"),
        ..Default::default()
    }))
    .map_err(|error| error.to_string())
}

/// Draws `recording` on the headless device and returns its RGBA bytes
/// (sRGB encoded, rows top-down).
pub fn read_back(recording: &Recording) -> Result<Vec<u8>, String> {
    let mut guard = HEADLESS.lock().map_err(|_| "headless lock poisoned")?;
    if guard.is_none() {
        let (device, queue) = open_device()?;
        let raster = GpuRaster::new(&device);
        *guard = Some(Headless {
            device,
            queue,
            raster,
        });
    }
    let headless = guard.as_mut().expect("headless device");
    let (width, height) = (recording.width.max(1), recording.height.max(1));
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let texture = headless.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tantalus-frame-gpu-readback"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    headless
        .raster
        .render(&headless.device, &headless.queue, recording, &view);
    let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = headless.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tantalus-frame-gpu-readback"),
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = headless
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        size,
    );
    headless.queue.submit(std::iter::once(encoder.finish()));
    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    headless
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|error| error.to_string())?;
    receiver
        .recv()
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let mapped = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for line in mapped.chunks(row as usize).take(height as usize) {
        rgba.extend_from_slice(&line[..(width * 4) as usize]);
    }
    drop(mapped);
    buffer.unmap();
    Ok(rgba)
}

pub use plugin::{FrameGpuPlugin, submit};

mod plugin {
    use super::{GpuRaster, Recording, TARGET_FORMAT};
    use bevy::prelude::*;
    use bevy::render::render_asset::RenderAssets;
    use bevy::render::renderer::{RenderDevice, RenderQueue};
    use bevy::render::texture::GpuImage;
    use bevy::render::{MainWorld, Render, RenderApp, RenderSystems};
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    /// Replays recorded frames into their images in the render world.
    pub struct FrameGpuPlugin;

    impl Plugin for FrameGpuPlugin {
        fn build(&self, app: &mut App) {
            if super::cpu_forced() {
                return;
            }
            app.init_resource::<FrameQueue>();
            let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
                return;
            };
            render_app
                .init_resource::<PendingFrames>()
                .add_systems(ExtractSchedule, extract_frames)
                .add_systems(Render, draw_frames.in_set(RenderSystems::PrepareResources));
            super::ACTIVE.store(true, Ordering::Relaxed);
        }
    }

    /// Frames waiting for the render world, by target image.
    #[derive(Resource, Default)]
    struct FrameQueue(Vec<(AssetId<Image>, Arc<Recording>)>);

    #[derive(Resource, Default)]
    struct PendingFrames(Vec<(AssetId<Image>, Arc<Recording>)>);

    /// Draws `recording` into `image` (made by
    /// [`crate::battle_view::frame_image`]) on the next render.
    pub fn submit(world: &mut World, image: &Handle<Image>, recording: Recording) {
        let id = image.id();
        if let Some(mut queue) = world.get_resource_mut::<FrameQueue>() {
            queue.0.retain(|(target, _)| *target != id);
            queue.0.push((id, Arc::new(recording)));
        }
    }

    fn extract_frames(mut main: ResMut<MainWorld>, mut pending: ResMut<PendingFrames>) {
        let Some(mut queue) = main.get_resource_mut::<FrameQueue>() else {
            return;
        };
        for (id, recording) in queue.0.drain(..) {
            pending.0.retain(|(target, _)| *target != id);
            pending.0.push((id, recording));
        }
    }

    fn draw_frames(
        mut pending: ResMut<PendingFrames>,
        images: Res<RenderAssets<GpuImage>>,
        device: Res<RenderDevice>,
        queue: Res<RenderQueue>,
        mut raster: Local<Option<GpuRaster>>,
    ) {
        if pending.0.is_empty() {
            return;
        }
        let device = device.wgpu_device();
        let raster = raster.get_or_insert_with(|| GpuRaster::new(device));
        // An image made this frame may not be on the GPU yet; its frame
        // waits for the next render.
        pending.0.retain(|(id, recording)| {
            let Some(image) = images.get(*id) else {
                return true;
            };
            let view =
                image
                    .texture
                    .create_view(&bevy::render::render_resource::TextureViewDescriptor {
                        label: Some("tantalus-frame-gpu-target"),
                        format: Some(TARGET_FORMAT),
                        ..Default::default()
                    });
            raster.render(device, &queue, recording, &view);
            false
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle_raster::{BattleCamera, Blend, DrawState, Frame, Texture, ViewVertex};
    use bevy::math::{Mat4, Vec3};

    /// The same draws on the CPU and recorded for the GPU land on nearly
    /// the same pixels (skipped where no GPU adapter answers).
    #[test]
    fn gpu_frame_matches_the_cpu_raster() {
        let camera = BattleCamera {
            view: Mat4::IDENTITY,
            near: 160.0,
        };
        let checker: Vec<u8> = (0..16 * 16)
            .flat_map(|i| {
                let on = (i % 16 + i / 16) % 2 == 0;
                if on {
                    [230, 40, 40, 255]
                } else {
                    [30, 30, 200, 255]
                }
            })
            .collect();
        let glow = [120u8, 200, 90, 255].repeat(4);
        let texture = Texture {
            width: 16,
            height: 16,
            rgba: &checker,
            clamp: false,
        };
        let glow = Texture {
            width: 2,
            height: 2,
            rgba: &glow,
            clamp: true,
        };
        let opaque = DrawState {
            blend: Blend::Opaque,
            depth_test: true,
            depth_write: true,
            cull_back: false,
        };
        let vertex = |x: f32, y: f32, z: f32, u: f32, v: f32| ViewVertex {
            position: Vec3::new(x, y, z),
            uv: [u, v],
        };
        let draw = |frame: &mut Frame| {
            // A far quad, a nearer tilted triangle through it, a screened
            // triangle over both, then the 2D overlays.
            let far = [
                vertex(-900.0, -700.0, -1000.0, 0.0, 0.0),
                vertex(900.0, -700.0, -1000.0, 3.0, 0.0),
                vertex(900.0, 900.0, -1000.0, 3.0, 3.0),
                vertex(-900.0, 900.0, -1000.0, 0.0, 3.0),
            ];
            frame.draw(&camera, [far[0], far[2], far[1]], Some(texture), opaque);
            frame.draw(&camera, [far[0], far[3], far[2]], Some(texture), opaque);
            frame.draw(
                &camera,
                [
                    vertex(-600.0, -500.0, -600.0, 0.0, 0.0),
                    vertex(0.0, 700.0, -1400.0, 0.5, 1.0),
                    vertex(700.0, -400.0, -900.0, 1.0, 0.0),
                ],
                Some(texture),
                opaque,
            );
            frame.draw(
                &camera,
                [
                    vertex(-300.0, -300.0, -500.0, 0.0, 0.0),
                    vertex(-300.0, 300.0, -500.0, 0.0, 1.0),
                    vertex(300.0, 0.0, -500.0, 1.0, 0.5),
                ],
                Some(glow),
                DrawState {
                    blend: Blend::Screen,
                    depth_test: true,
                    depth_write: false,
                    cull_back: false,
                },
            );
            frame.rect(10, 10, 60, 20, [0, 0, 0], 0.5);
            frame.blit(80, 10, 4, 2, &[250u8, 250, 250, 255].repeat(8));
            frame.text(4, 100, "HP 42", 2, [255, 255, 255]);
        };
        let mut cpu = Frame::new(160, 120, [10, 20, 30, 255]);
        draw(&mut cpu);
        let mut gpu = Frame::recorded(160, 120, [10, 20, 30, 255]);
        draw(&mut gpu);
        let pixels = match read_back(gpu.recording().expect("recorded")) {
            Ok(pixels) => pixels,
            Err(error) => {
                eprintln!("no GPU, skipped: {error}");
                return;
            }
        };
        assert_eq!(pixels.len(), cpu.rgba.len());
        let differing = pixels
            .chunks(4)
            .zip(cpu.rgba.chunks(4))
            .filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 24))
            .count();
        // Edges may round to the other side.
        assert!(
            differing < 160,
            "{differing} of {} pixels differ",
            160 * 120
        );
        assert!(cpu.rgba.chunks(4).any(|p| p[0] > 200));
    }
}
