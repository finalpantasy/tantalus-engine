//! CPU frame for the battle view: the battle camera, a depth-buffered
//! perspective-correct triangle rasterizer with the battle map blend modes,
//! and a 5x7 debug font for the HP/ATB readout.
//!
//! Camera (Memoria `BattleMapCameraController.SetDefaultPsxCamera0..2`):
//! three default Unity transforms, one per `PatAddr[pattern].Camera`
//! (`HonoluluBattleMain.Start`: 3 and up pick one at random). The
//! projection is `PsxCamera.PsxProj2UnityProj(near, far)`, the off-centre
//! frustum the battle camera always gets from `SFX.UpdateCamera`: x spans
//! -160..160 and y -100..120 at distance `near` (`PsxScreenHeightNative`
//! 220, bottom `220 / 2.2`). The preset's `nearClipPlane` (400, 360, 300)
//! stands in for the plugin's projection distance (`SFX_UpdateCamera`
//! result 12), which lives in the native `FF9SpecialEffectPlugin.dll`.

use bevy::math::{EulerRot, Mat4, Quat, Vec3};

/// PSX screen the projection spans.
pub const PSX_WIDTH: f32 = 320.0;
pub const PSX_HEIGHT: f32 = 220.0;
/// `PsxProj2UnityProj`: `top = 220 - 220 / 2.2`.
const PSX_TOP: f32 = PSX_HEIGHT - PSX_HEIGHT / 2.2;
/// Nearest view distance drawn (Memoria sets `nearClipPlane` 0.1).
const NEAR_CLIP: f32 = 1.0;

/// One of the default battle cameras.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BattleCamera {
    /// World to view (Unity camera space: x right, y up, looking down -z).
    pub view: Mat4,
    /// Projection distance.
    pub near: f32,
}

impl BattleCamera {
    /// `SetDefaultPsxCamera<preset>`; presets past 2 use camera 0.
    pub fn preset(preset: u8) -> Self {
        let (position, euler, near) = match preset {
            1 => ([-2100.0, 900.0, -4500.0], [3.0, 30.0, 0.0], 360.0),
            2 => ([0.0, 1000.0, -4500.0], [10.0, 0.0, 0.0], 300.0),
            _ => ([-2200.0, 2900.0, 4600.0], [25.0, 155.0, 0.0], 400.0),
        };
        Self::from_transform(Vec3::from_array(position), euler, near)
    }

    /// A Unity camera at `position` turned `Quaternion.Euler(euler)`
    /// (degrees, applied z, then x, then y).
    pub fn from_transform(position: Vec3, euler: [f32; 3], near: f32) -> Self {
        let rotation = Quat::from_euler(
            EulerRot::YXZ,
            euler[1].to_radians(),
            euler[0].to_radians(),
            euler[2].to_radians(),
        );
        // `worldToCameraMatrix`: the inverse camera transform with z negated
        // (OpenGL convention, the camera looks down -z).
        let camera = Mat4::from_rotation_translation(rotation, position);
        let view = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0)) * camera.inverse();
        Self { view, near }
    }

    pub fn to_view(&self, world: [f32; 3]) -> Vec3 {
        self.view.transform_point3(Vec3::from_array(world))
    }

    /// World to clip space for the GPU frame ([`crate::frame_gpu`]): x and
    /// y as [`Frame::project`] places them, w the view distance, and z 1, so
    /// depth is 1 / distance and the GPU's `z <= w` clip is the CPU's
    /// `distance >= NEAR_CLIP` (1).
    pub fn clip_matrix(&self) -> Mat4 {
        use bevy::math::Vec4;
        let half = PSX_WIDTH / 2.0;
        let offset = 1.0 - 2.0 * PSX_TOP / PSX_HEIGHT;
        let projection = Mat4::from_cols(
            Vec4::new(self.near / half, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 2.0 * self.near / PSX_HEIGHT, 0.0, 0.0),
            Vec4::new(0.0, -offset, 0.0, -1.0),
            Vec4::new(0.0, 0.0, 1.0, 0.0),
        );
        projection * self.view
    }
}

/// A view-space vertex with its texture coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewVertex {
    pub position: Vec3,
    pub uv: [f32; 2],
}

/// The frame as it was (see [`Frame::capture`]).
#[derive(Debug, Clone)]
pub struct Capture {
    pub width: usize,
    pub height: usize,
    /// The pixels; empty for a GPU capture.
    pub rgba: Vec<u8>,
    /// The recording's texture index of a GPU capture.
    pub gpu: Option<u32>,
}

/// RGBA texels, rows top-down, `v = 0` the last row.
#[derive(Debug, Clone, Copy)]
pub struct Texture<'a> {
    pub width: usize,
    pub height: usize,
    pub rgba: &'a [u8],
    /// Clamp (battle maps, `TextureWrapMode.Clamp`) or repeat.
    pub clamp: bool,
}

impl Texture<'_> {
    fn sample(&self, uv: [f32; 2]) -> [u8; 4] {
        let coordinate = |value: f32, size: usize| {
            let value = if self.clamp {
                value.clamp(0.0, 1.0)
            } else {
                value - value.floor()
            };
            ((value * size as f32) as usize).min(size - 1)
        };
        let x = coordinate(uv[0], self.width);
        let y = self.height - 1 - coordinate(uv[1], self.height);
        let at = (y * self.width + x) * 4;
        self.rgba
            .get(at..at + 4)
            .map_or([255, 0, 255, 255], |texel| {
                [texel[0], texel[1], texel[2], texel[3]]
            })
    }
}

/// How a triangle writes its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    /// Cut out under alpha 0.5, then replace.
    Opaque,
    /// `Blend One OneMinusSrcColor`.
    Screen,
    /// Darkens: `dst * src`.
    Multiply,
    /// Straight alpha: texel alpha times this opacity (0 to 255) over the
    /// pixel (the world's water).
    Alpha(u8),
    /// `Blend SrcAlpha One`: the pixel plus the color times texel alpha
    /// (the special effects' additive meshes).
    Add,
    /// `Blend One One`, `BlendOp RevSub`: the pixel less the color.
    Subtract,
}

/// A corner of a screen-space triangle ([`Frame::draw_screen`]): frame
/// pixels, texture coordinate (as [`Texture`] reads it: `v = 0` is the
/// last row) and the color the texel is multiplied by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenCorner {
    pub xy: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawState {
    pub blend: Blend,
    pub depth_test: bool,
    pub depth_write: bool,
    /// Unity's default `Cull Back`: only triangles that wind clockwise on
    /// screen are drawn.
    pub cull_back: bool,
}

/// Color and depth buffers.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// View distance per pixel (`f32::INFINITY` where nothing wrote).
    pub depth: Vec<f32>,
    /// Set for a frame drawn on the GPU ([`Frame::for_view`]): the draws
    /// are recorded instead, and `rgba` and `depth` stay empty.
    recording: Option<Box<crate::frame_gpu::Recording>>,
}

#[derive(Clone, Copy)]
struct ScreenVertex {
    xy: [f32; 2],
    /// 1 / view distance.
    w: f32,
    /// uv / view distance.
    uv: [f32; 2],
}

impl Frame {
    pub fn new(width: u32, height: u32, clear: [u8; 4]) -> Self {
        let pixels = (width * height) as usize;
        Self {
            width,
            height,
            rgba: clear.repeat(pixels),
            depth: vec![f32::INFINITY; pixels],
            recording: None,
        }
    }

    /// A frame of finished pixels (a movie frame), without depth.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        Self {
            width,
            height,
            rgba,
            depth: Vec::new(),
            recording: None,
        }
    }

    /// A frame for the battle or world view: recorded for the GPU when
    /// [`crate::frame_gpu::active`], drawn on the CPU otherwise.
    pub fn for_view(width: u32, height: u32, clear: [u8; 4]) -> Self {
        if crate::frame_gpu::active() {
            Self::recorded(width, height, clear)
        } else {
            Self::new(width, height, clear)
        }
    }

    /// A frame recorded for the GPU whether or not the backend is on.
    pub fn recorded(width: u32, height: u32, clear: [u8; 4]) -> Self {
        Self {
            width,
            height,
            rgba: Vec::new(),
            depth: Vec::new(),
            recording: Some(Box::new(crate::frame_gpu::Recording::new(
                width, height, clear,
            ))),
        }
    }

    /// The draws of a GPU frame.
    pub fn recording(&self) -> Option<&crate::frame_gpu::Recording> {
        self.recording.as_deref()
    }

    pub fn recording_mut(&mut self) -> Option<&mut crate::frame_gpu::Recording> {
        self.recording.as_deref_mut()
    }

    pub fn into_recording(self) -> Option<crate::frame_gpu::Recording> {
        self.recording.map(|recording| *recording)
    }

    /// Screen position of a view-space point in front of the camera.
    pub fn project(&self, camera: &BattleCamera, view: Vec3) -> Option<[f32; 2]> {
        let distance = -view.z;
        (distance >= NEAR_CLIP).then(|| {
            let x = PSX_WIDTH / 2.0 + camera.near * view.x / distance;
            let y = PSX_TOP - camera.near * view.y / distance;
            [
                x * self.width as f32 / PSX_WIDTH,
                y * self.height as f32 / PSX_HEIGHT,
            ]
        })
    }

    /// Clips one view-space triangle at the near plane and draws it.
    pub fn draw(
        &mut self,
        camera: &BattleCamera,
        triangle: [ViewVertex; 3],
        texture: Option<Texture<'_>>,
        state: DrawState,
    ) {
        // Sutherland-Hodgman against `distance >= NEAR_CLIP`.
        let inside = |vertex: &ViewVertex| -vertex.position.z >= NEAR_CLIP;
        let mut polygon: Vec<ViewVertex> = Vec::with_capacity(4);
        for index in 0..3 {
            let a = triangle[index];
            let b = triangle[(index + 1) % 3];
            if inside(&a) {
                polygon.push(a);
            }
            if inside(&a) != inside(&b) {
                let (da, db) = (-a.position.z - NEAR_CLIP, -b.position.z - NEAR_CLIP);
                let t = da / (da - db);
                polygon.push(ViewVertex {
                    position: a.position.lerp(b.position, t),
                    uv: [
                        a.uv[0] + (b.uv[0] - a.uv[0]) * t,
                        a.uv[1] + (b.uv[1] - a.uv[1]) * t,
                    ],
                });
            }
        }
        if polygon.len() < 3 {
            return;
        }
        let screen: Vec<ScreenVertex> = polygon
            .iter()
            .filter_map(|vertex| {
                let xy = self.project(camera, vertex.position)?;
                let w = 1.0 / -vertex.position.z;
                Some(ScreenVertex {
                    xy,
                    w,
                    uv: [vertex.uv[0] * w, vertex.uv[1] * w],
                })
            })
            .collect();
        for index in 1..screen.len().saturating_sub(1) {
            self.fill(
                [screen[0], screen[index], screen[index + 1]],
                texture,
                state,
            );
        }
    }

    fn fill(&mut self, v: [ScreenVertex; 3], texture: Option<Texture<'_>>, state: DrawState) {
        if self.recording.is_some() {
            self.record_fill(v, texture, state);
        } else {
            self.fill_with(v, texture, state, true);
        }
    }

    /// [`Frame::fill_with`] for the GPU: the same cull, then the triangle
    /// in clip space with w the view distance (so the GPU interpolates uv
    /// as the CPU does) and depth 1 / distance.
    fn record_fill(
        &mut self,
        v: [ScreenVertex; 3],
        texture: Option<Texture<'_>>,
        state: DrawState,
    ) {
        use crate::frame_gpu::Sampling;
        let area = edge(v[0].xy, v[1].xy, v[2].xy);
        if area.abs() < 1e-6 || (state.cull_back && area < 0.0) {
            return;
        }
        let Some(recording) = self.recording.as_deref_mut() else {
            return;
        };
        let index = recording.texture(texture.map(|t| (t.rgba, t.width, t.height)));
        let sampling = match texture {
            Some(texture) if !texture.clamp => Sampling::Repeat,
            _ => Sampling::Clamp,
        };
        // Without a texture the CPU draws light gray.
        let shade = if texture.is_some() {
            1.0
        } else {
            200.0 / 255.0
        };
        // Culled above.
        let (key, alpha) = gpu_state(DrawState {
            cull_back: false,
            ..state
        });
        let vertices = v.map(|vertex| {
            let distance = 1.0 / vertex.w;
            let (x, y) = recording.ndc(vertex.xy[0], vertex.xy[1]);
            [
                x * distance,
                y * distance,
                1.0,
                distance,
                vertex.uv[0] * distance,
                // Texel rows run top-down on the GPU; `v = 0` is the last.
                1.0 - vertex.uv[1] * distance,
                shade,
                shade,
                shade,
                alpha,
                0.0,
                0.0,
            ]
        });
        recording.push(key, index, sampling, &vertices);
    }

    /// `narrow` limits each row to the columns the triangle can cover; off,
    /// the whole bounding box is scanned (the reference the tests compare to).
    fn fill_with(
        &mut self,
        v: [ScreenVertex; 3],
        texture: Option<Texture<'_>>,
        state: DrawState,
        narrow: bool,
    ) {
        let area = edge(v[0].xy, v[1].xy, v[2].xy);
        // Screen y points down, so a clockwise triangle has a positive area.
        if area.abs() < 1e-6 || (state.cull_back && area < 0.0) {
            return;
        }
        let min_x = v.iter().map(|p| p.xy[0]).fold(f32::INFINITY, f32::min);
        let max_x = v.iter().map(|p| p.xy[0]).fold(f32::NEG_INFINITY, f32::max);
        let min_y = v.iter().map(|p| p.xy[1]).fold(f32::INFINITY, f32::min);
        let max_y = v.iter().map(|p| p.xy[1]).fold(f32::NEG_INFINITY, f32::max);
        let x0 = min_x.floor().max(0.0) as i64;
        let x1 = (max_x.ceil() as i64).min(i64::from(self.width) - 1);
        let y0 = min_y.floor().max(0.0) as i64;
        let y1 = (max_y.ceil() as i64).min(i64::from(self.height) - 1);
        let sign = if area < 0.0 { -1.0 } else { 1.0 };
        let sides = [(v[1].xy, v[2].xy), (v[2].xy, v[0].xy), (v[0].xy, v[1].xy)];
        for y in y0..=y1 {
            // Only columns near where all three edge tests can hold are
            // shaded; the per-pixel tests below decide, so the result is the
            // same as scanning the whole bounding box.
            let (mut from, mut to) = (x0, x1);
            let py = y as f32 + 0.5;
            let at = x0 as f32 + 0.5;
            for (a, b) in sides.into_iter().filter(|_| narrow) {
                let f0 = sign * edge(a, b, [at, py]);
                let slope = sign * edge(a, b, [at + 1.0, py]) - f0;
                if slope > 1e-12 {
                    from = from.max(x0 + (-f0 / slope).floor() as i64 - 2);
                } else if slope < -1e-12 {
                    to = to.min(x0 + (-f0 / slope).ceil() as i64 + 2);
                }
            }
            for x in from..=to {
                let p = [x as f32 + 0.5, y as f32 + 0.5];
                let b0 = edge(v[1].xy, v[2].xy, p) / area;
                let b1 = edge(v[2].xy, v[0].xy, p) / area;
                let b2 = 1.0 - b0 - b1;
                if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                    continue;
                }
                let w = b0 * v[0].w + b1 * v[1].w + b2 * v[2].w;
                if w <= 0.0 {
                    continue;
                }
                let distance = 1.0 / w;
                let at = (y as usize) * self.width as usize + x as usize;
                if state.depth_test && distance >= self.depth[at] {
                    continue;
                }
                let texel = match texture {
                    Some(texture) => texture.sample([
                        (b0 * v[0].uv[0] + b1 * v[1].uv[0] + b2 * v[2].uv[0]) * distance,
                        (b0 * v[0].uv[1] + b1 * v[1].uv[1] + b2 * v[2].uv[1]) * distance,
                    ]),
                    None => [200, 200, 200, 255],
                };
                let pixel = &mut self.rgba[at * 4..at * 4 + 4];
                match state.blend {
                    Blend::Opaque => {
                        if texel[3] < 128 {
                            continue;
                        }
                        pixel.copy_from_slice(&[texel[0], texel[1], texel[2], 255]);
                    }
                    Blend::Screen => {
                        for channel in 0..3 {
                            let (src, dst) = (u32::from(texel[channel]), u32::from(pixel[channel]));
                            pixel[channel] = (src + dst * (255 - src) / 255).min(255) as u8;
                        }
                    }
                    Blend::Alpha(opacity) => {
                        let alpha = u32::from(texel[3]) * u32::from(opacity) / 255;
                        for channel in 0..3 {
                            let (src, dst) = (u32::from(texel[channel]), u32::from(pixel[channel]));
                            pixel[channel] = ((src * alpha + dst * (255 - alpha)) / 255) as u8;
                        }
                    }
                    Blend::Multiply => {
                        for channel in 0..3 {
                            pixel[channel] =
                                (u32::from(pixel[channel]) * u32::from(texel[channel]) / 255) as u8;
                        }
                    }
                    Blend::Add => {
                        for channel in 0..3 {
                            let (src, dst) = (u32::from(texel[channel]), u32::from(pixel[channel]));
                            pixel[channel] = (dst + src * u32::from(texel[3]) / 255).min(255) as u8;
                        }
                    }
                    Blend::Subtract => {
                        for channel in 0..3 {
                            pixel[channel] = pixel[channel].saturating_sub(texel[channel]);
                        }
                    }
                }
                if state.depth_write {
                    self.depth[at] = distance;
                }
            }
        }
    }

    /// What the frame shows so far, for draws that sample the screen
    /// ([`Frame::draw_captured`]): a copy of the pixels, or on the GPU a
    /// capture made when the recording reaches this point.
    pub fn capture(&mut self) -> Option<Capture> {
        if let Some(recording) = self.recording.as_deref_mut() {
            return Some(Capture {
                width: self.width as usize,
                height: self.height as usize,
                rgba: Vec::new(),
                gpu: Some(recording.capture()),
            });
        }
        Some(Capture {
            width: self.width as usize,
            height: self.height as usize,
            rgba: self.rgba.clone(),
            gpu: None,
        })
    }

    /// [`Frame::draw_screen`] sampling a [`Frame::capture`]. Texel
    /// coordinates are 0 to 1 over the frame, `v = 0` the last row.
    pub fn draw_captured(&mut self, corners: [ScreenCorner; 3], capture: &Capture, blend: Blend) {
        if let (Some(index), Some(recording)) = (capture.gpu, self.recording.as_deref_mut()) {
            use crate::frame_gpu::{BlendKind, PipelineKey, Sampling};
            if edge(corners[0].xy, corners[1].xy, corners[2].xy).abs() < 1e-6 {
                return;
            }
            let kind = match blend {
                Blend::Opaque => BlendKind::Replace,
                Blend::Screen => BlendKind::Screen,
                Blend::Multiply => BlendKind::Multiply,
                Blend::Alpha(_) => BlendKind::Over,
                Blend::Add => BlendKind::Add,
                Blend::Subtract => BlendKind::Subtract,
            };
            let alpha = match blend {
                Blend::Alpha(opacity) => f32::from(opacity) / 255.0,
                _ => 1.0,
            };
            let key = PipelineKey {
                blend: kind,
                depth_test: false,
                depth_write: false,
                cull_back: false,
            };
            let vertices = corners.map(|corner| {
                let [r, g, b] = corner.color;
                recording.flat(
                    corner.xy[0],
                    corner.xy[1],
                    [corner.uv[0], 1.0 - corner.uv[1]],
                    [r, g, b, alpha],
                    [0.0, 0.0],
                )
            });
            recording.push(key, index, Sampling::Clamp, &vertices);
            return;
        }
        let texture = Texture {
            width: capture.width,
            height: capture.height,
            rgba: &capture.rgba,
            clamp: true,
        };
        self.draw_screen(corners, Some(texture), blend);
    }

    /// A triangle already on screen (the special effects' packets), drawn
    /// over everything: no depth, no culling, affine texture coordinates.
    /// Without a texture the color itself is drawn. `Blend::Opaque` cuts
    /// out texels under alpha 0.5.
    pub fn draw_screen(
        &mut self,
        corners: [ScreenCorner; 3],
        texture: Option<Texture<'_>>,
        blend: Blend,
    ) {
        let area = edge(corners[0].xy, corners[1].xy, corners[2].xy);
        if area.abs() < 1e-6 {
            return;
        }
        if let Some(recording) = self.recording.as_deref_mut() {
            use crate::frame_gpu::{BlendKind, PipelineKey, Sampling};
            let index = recording.texture(texture.map(|t| (t.rgba, t.width, t.height)));
            let kind = match blend {
                Blend::Opaque => BlendKind::Replace,
                Blend::Screen => BlendKind::Screen,
                Blend::Multiply => BlendKind::Multiply,
                Blend::Alpha(_) => BlendKind::Over,
                Blend::Add => BlendKind::Add,
                Blend::Subtract => BlendKind::Subtract,
            };
            let alpha = match blend {
                Blend::Alpha(opacity) => f32::from(opacity) / 255.0,
                _ => 1.0,
            };
            let key = PipelineKey {
                blend: kind,
                depth_test: false,
                depth_write: false,
                cull_back: false,
            };
            let vertices = corners.map(|corner| {
                let [r, g, b] = corner.color;
                // Texel rows run top-down on the GPU; `v = 0` is the last.
                recording.flat(
                    corner.xy[0],
                    corner.xy[1],
                    [corner.uv[0], 1.0 - corner.uv[1]],
                    [r, g, b, alpha],
                    [0.0, 0.0],
                )
            });
            recording.push(key, index, Sampling::Clamp, &vertices);
            return;
        }
        let xs = corners.map(|corner| corner.xy[0]);
        let ys = corners.map(|corner| corner.xy[1]);
        let x0 = xs
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as i64;
        let x1 = (xs.iter().copied().fold(f32::NEG_INFINITY, f32::max).ceil() as i64)
            .min(i64::from(self.width) - 1);
        let y0 = ys
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as i64;
        let y1 = (ys.iter().copied().fold(f32::NEG_INFINITY, f32::max).ceil() as i64)
            .min(i64::from(self.height) - 1);
        let [a, b, c] = corners;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = [x as f32 + 0.5, y as f32 + 0.5];
                let b0 = edge(b.xy, c.xy, p) / area;
                let b1 = edge(c.xy, a.xy, p) / area;
                let b2 = 1.0 - b0 - b1;
                if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                    continue;
                }
                let texel = match texture {
                    Some(texture) => texture.sample([
                        b0 * a.uv[0] + b1 * b.uv[0] + b2 * c.uv[0],
                        b0 * a.uv[1] + b1 * b.uv[1] + b2 * c.uv[1],
                    ]),
                    None => [255, 255, 255, 255],
                };
                if blend == Blend::Opaque && texel[3] < 128 {
                    continue;
                }
                let at = ((y as usize) * self.width as usize + x as usize) * 4;
                let pixel = &mut self.rgba[at..at + 4];
                for channel in 0..3 {
                    let shade =
                        b0 * a.color[channel] + b1 * b.color[channel] + b2 * c.color[channel];
                    let src = (f32::from(texel[channel]) * shade).clamp(0.0, 255.0);
                    let dst = f32::from(pixel[channel]);
                    let value = match blend {
                        Blend::Add => dst + src * f32::from(texel[3]) / 255.0,
                        Blend::Subtract => dst - src,
                        Blend::Alpha(opacity) => {
                            let alpha = f32::from(texel[3]) * f32::from(opacity) / (255.0 * 255.0);
                            src * alpha + dst * (1.0 - alpha)
                        }
                        Blend::Multiply => dst * src / 255.0,
                        Blend::Screen => src + dst * (255.0 - src) / 255.0,
                        Blend::Opaque => src,
                    };
                    pixel[channel] = value.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    /// Copies `rgba` (opaque, `width` x `height`) with its top left at
    /// `(x, y)`.
    pub fn blit(&mut self, x: i32, y: i32, width: i32, height: i32, rgba: &[u8]) {
        if width <= 0 || height <= 0 || rgba.len() < (width * height * 4) as usize {
            return;
        }
        if let Some(recording) = self.recording.as_deref_mut() {
            recording.blit(x, y, width, height, rgba);
            return;
        }
        for row in y.max(0)..(y + height).min(self.height as i32) {
            for column in x.max(0)..(x + width).min(self.width as i32) {
                let at = (row as usize * self.width as usize + column as usize) * 4;
                let from = (((row - y) * width + (column - x)) * 4) as usize;
                self.rgba[at..at + 3].copy_from_slice(&rgba[from..from + 3]);
            }
        }
    }

    /// A filled rectangle blended at `alpha`.
    pub fn rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: [u8; 3], alpha: f32) {
        if let Some(recording) = self.recording.as_deref_mut() {
            recording.rect(x, y, width, height, color, alpha);
            return;
        }
        for row in y.max(0)..(y + height).min(self.height as i32) {
            for column in x.max(0)..(x + width).min(self.width as i32) {
                let at = (row as usize * self.width as usize + column as usize) * 4;
                for (pixel, source) in self.rgba[at..at + 3].iter_mut().zip(color) {
                    let dst = f32::from(*pixel);
                    *pixel = (dst + (f32::from(source) - dst) * alpha).round() as u8;
                }
            }
        }
    }

    /// `text` in the 5x7 font, `scale` pixels per font pixel, with a dark
    /// drop shadow. Returns the width drawn.
    pub fn text(&mut self, x: i32, y: i32, text: &str, scale: i32, color: [u8; 3]) -> i32 {
        for (offset, shade) in [(scale.max(1) / 2 + 1, [16, 16, 24]), (0, color)] {
            let mut pen = x + offset;
            for character in text.chars() {
                let glyph = glyph(character);
                for (column, bits) in glyph.iter().enumerate() {
                    for row in 0..7 {
                        if bits >> row & 1 != 0 {
                            self.rect(
                                pen + column as i32 * scale,
                                y + offset + row * scale,
                                scale,
                                scale,
                                shade,
                                1.0,
                            );
                        }
                    }
                }
                pen += 6 * scale;
            }
        }
        text.chars().count() as i32 * 6 * scale
    }
}

/// `state` as a GPU pipeline, and the vertex alpha its blend takes.
pub fn gpu_state(state: DrawState) -> (crate::frame_gpu::PipelineKey, f32) {
    use crate::frame_gpu::{BlendKind, PipelineKey};
    let (blend, alpha) = match state.blend {
        Blend::Opaque => (BlendKind::Replace, 1.0),
        Blend::Screen => (BlendKind::Screen, 1.0),
        Blend::Multiply => (BlendKind::Multiply, 1.0),
        Blend::Alpha(opacity) => (BlendKind::Over, f32::from(opacity) / 255.0),
        Blend::Add => (BlendKind::Add, 1.0),
        Blend::Subtract => (BlendKind::Subtract, 1.0),
    };
    (
        PipelineKey {
            blend,
            depth_test: state.depth_test,
            depth_write: state.depth_write,
            cull_back: state.cull_back,
        },
        alpha,
    )
}

fn edge(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Columns of a 5x7 glyph (bit 0 the top row); lower case shows as upper.
fn glyph(character: char) -> [u8; 5] {
    let code = character.to_ascii_uppercase() as usize;
    FONT.get(code.wrapping_sub(0x20))
        .copied()
        .unwrap_or(FONT[(b'?' - 0x20) as usize])
}

/// The classic 5x7 terminal font, ASCII 0x20..=0x5F, column-major.
const FONT: [[u8; 5]; 64] = [
    [0x00, 0x00, 0x00, 0x00, 0x00],
    [0x00, 0x00, 0x5F, 0x00, 0x00],
    [0x00, 0x07, 0x00, 0x07, 0x00],
    [0x14, 0x7F, 0x14, 0x7F, 0x14],
    [0x24, 0x2A, 0x7F, 0x2A, 0x12],
    [0x23, 0x13, 0x08, 0x64, 0x62],
    [0x36, 0x49, 0x55, 0x22, 0x50],
    [0x00, 0x05, 0x03, 0x00, 0x00],
    [0x00, 0x1C, 0x22, 0x41, 0x00],
    [0x00, 0x41, 0x22, 0x1C, 0x00],
    [0x08, 0x2A, 0x1C, 0x2A, 0x08],
    [0x08, 0x08, 0x3E, 0x08, 0x08],
    [0x00, 0x50, 0x30, 0x00, 0x00],
    [0x08, 0x08, 0x08, 0x08, 0x08],
    [0x00, 0x60, 0x60, 0x00, 0x00],
    [0x20, 0x10, 0x08, 0x04, 0x02],
    [0x3E, 0x51, 0x49, 0x45, 0x3E],
    [0x00, 0x42, 0x7F, 0x40, 0x00],
    [0x42, 0x61, 0x51, 0x49, 0x46],
    [0x21, 0x41, 0x45, 0x4B, 0x31],
    [0x18, 0x14, 0x12, 0x7F, 0x10],
    [0x27, 0x45, 0x45, 0x45, 0x39],
    [0x3C, 0x4A, 0x49, 0x49, 0x30],
    [0x01, 0x71, 0x09, 0x05, 0x03],
    [0x36, 0x49, 0x49, 0x49, 0x36],
    [0x06, 0x49, 0x49, 0x29, 0x1E],
    [0x00, 0x36, 0x36, 0x00, 0x00],
    [0x00, 0x56, 0x36, 0x00, 0x00],
    [0x00, 0x08, 0x14, 0x22, 0x41],
    [0x14, 0x14, 0x14, 0x14, 0x14],
    [0x41, 0x22, 0x14, 0x08, 0x00],
    [0x02, 0x01, 0x51, 0x09, 0x06],
    [0x32, 0x49, 0x79, 0x41, 0x3E],
    [0x7E, 0x11, 0x11, 0x11, 0x7E],
    [0x7F, 0x49, 0x49, 0x49, 0x36],
    [0x3E, 0x41, 0x41, 0x41, 0x22],
    [0x7F, 0x41, 0x41, 0x22, 0x1C],
    [0x7F, 0x49, 0x49, 0x49, 0x41],
    [0x7F, 0x09, 0x09, 0x01, 0x01],
    [0x3E, 0x41, 0x41, 0x51, 0x32],
    [0x7F, 0x08, 0x08, 0x08, 0x7F],
    [0x00, 0x41, 0x7F, 0x41, 0x00],
    [0x20, 0x40, 0x41, 0x3F, 0x01],
    [0x7F, 0x08, 0x14, 0x22, 0x41],
    [0x7F, 0x40, 0x40, 0x40, 0x40],
    [0x7F, 0x02, 0x04, 0x02, 0x7F],
    [0x7F, 0x04, 0x08, 0x10, 0x7F],
    [0x3E, 0x41, 0x41, 0x41, 0x3E],
    [0x7F, 0x09, 0x09, 0x09, 0x06],
    [0x3E, 0x41, 0x51, 0x21, 0x5E],
    [0x7F, 0x09, 0x19, 0x29, 0x46],
    [0x46, 0x49, 0x49, 0x49, 0x31],
    [0x01, 0x01, 0x7F, 0x01, 0x01],
    [0x3F, 0x40, 0x40, 0x40, 0x3F],
    [0x1F, 0x20, 0x40, 0x20, 0x1F],
    [0x7F, 0x20, 0x18, 0x20, 0x7F],
    [0x63, 0x14, 0x08, 0x14, 0x63],
    [0x03, 0x04, 0x78, 0x04, 0x03],
    [0x61, 0x51, 0x49, 0x45, 0x43],
    [0x00, 0x00, 0x7F, 0x41, 0x41],
    [0x02, 0x04, 0x08, 0x10, 0x20],
    [0x41, 0x41, 0x7F, 0x00, 0x00],
    [0x04, 0x02, 0x01, 0x02, 0x04],
    [0x40, 0x40, 0x40, 0x40, 0x40],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_zero_looks_at_the_battlefield_centre() {
        let camera = BattleCamera::preset(0);
        let centre = camera.to_view([0.0, 0.0, 0.0]);
        // In front of the camera, near the middle of the screen.
        assert!(centre.z < -5000.0, "{centre:?}");
        let frame = Frame::new(320, 220, [0; 4]);
        let [x, y] = frame.project(&camera, centre).expect("in front");
        assert!(
            (x - 160.0).abs() < 60.0 && (y - 110.0).abs() < 80.0,
            "{x} {y}"
        );
    }

    #[test]
    fn up_in_the_world_is_up_on_screen() {
        let camera = BattleCamera::preset(2);
        let frame = Frame::new(320, 220, [0; 4]);
        let low = frame.project(&camera, camera.to_view([0.0, 0.0, 0.0]));
        let high = frame.project(&camera, camera.to_view([0.0, 500.0, 0.0]));
        assert!(high.expect("high")[1] < low.expect("low")[1]);
    }

    #[test]
    fn nearer_triangles_win_and_clipped_ones_still_draw() {
        let camera = BattleCamera {
            view: Mat4::IDENTITY,
            near: 160.0,
        };
        let mut frame = Frame::new(32, 22, [0; 4]);
        let solid = [255u8, 0, 0, 255];
        let texture = Texture {
            width: 1,
            height: 1,
            rgba: &solid,
            clamp: true,
        };
        let state = DrawState {
            blend: Blend::Opaque,
            depth_test: true,
            depth_write: true,
            cull_back: false,
        };
        let quad = |z: f32| {
            [
                ViewVertex {
                    position: Vec3::new(-1000.0, -1000.0, z),
                    uv: [0.0, 0.0],
                },
                ViewVertex {
                    position: Vec3::new(1000.0, -1000.0, z),
                    uv: [1.0, 0.0],
                },
                ViewVertex {
                    position: Vec3::new(0.0, 1000.0, 5.0),
                    uv: [0.5, 1.0],
                },
            ]
        };
        frame.draw(&camera, quad(-100.0), Some(texture), state);
        assert!(frame.rgba.chunks(4).any(|pixel| pixel[0] == 255));
        assert!(frame.depth.iter().any(|depth| depth.is_finite()));
    }

    #[test]
    fn text_draws_pixels() {
        let mut frame = Frame::new(40, 10, [0; 4]);
        let width = frame.text(0, 0, "HP 9", 1, [255, 255, 255]);
        assert_eq!(width, 24);
        assert!(frame.rgba.chunks(4).any(|pixel| pixel[0] == 255));
    }
}

#[cfg(test)]
mod narrowing_tests {
    use super::*;

    /// Narrowed rows shade exactly the pixels a bounding-box scan does.
    #[test]
    fn narrowed_fill_matches_the_bounding_box_scan() {
        let mut seed = 0x1234_5678_u32;
        let mut next = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / 16_777_216.0
        };
        let mut narrow = Frame::new(64, 48, [0, 0, 0, 255]);
        let mut full = Frame::new(64, 48, [0, 0, 0, 255]);
        for _ in 0..400 {
            let vertex = |next: &mut dyn FnMut() -> f32| ScreenVertex {
                xy: [next() * 90.0 - 15.0, next() * 70.0 - 11.0],
                w: 0.01 + next() * 0.5,
                uv: [0.0, 0.0],
            };
            let v = [vertex(&mut next), vertex(&mut next), vertex(&mut next)];
            let state = DrawState {
                blend: Blend::Opaque,
                depth_test: true,
                depth_write: true,
                cull_back: next() < 0.5,
            };
            narrow.fill_with(v, None, state, true);
            full.fill_with(v, None, state, false);
        }
        assert_eq!(narrow.rgba, full.rgba);
        assert_eq!(narrow.depth, full.depth);
        assert!(narrow.rgba.iter().any(|&byte| byte != 0));
    }
}
