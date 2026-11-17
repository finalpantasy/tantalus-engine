//! A frame's GPU packets (`SFX_GetPrim`) as the Steam game draws them:
//! Memoria's `SFXRender.Add` and `SFXMesh` (`Global/SFXRender`,
//! `Global/SFXMesh`), the vanilla PC renderer.
//!
//! The PC port does not draw the packets in ordering-table order. It sorts
//! them into meshes by blend (opaque, additive, subtractive), texture page
//! and CLUT, then draws the meshes in a fixed order (`PushCommandBuffer`:
//! opaque, additive and subtractive, with the subtractive ones first,
//! second or last as the effect's `playParam` says). A `DR_MOVE` or
//! `DR_AREA` packet closes the batch. Its shaders (`SFX_OPA_GT`,
//! `SFX_ADD_GT`, `SFX_SUB_GT` and the untextured `_G` ones) multiply the
//! texel by the vertex color, the effect's color intensity
//! (`SFXMesh.ColorData`: 1, 1.5 or 2) and the vertex alpha, which carries
//! the PSX blend weight (`AbrAlphaData`: 63, 127, 127, 31 of 255, doubled
//! for modulated textures by `GetShadeAlpha`). Additive meshes blend
//! `SrcAlpha One` (so PSX mode 0, half back plus half front, becomes
//! back plus half front), subtractive ones `One One RevSub`.
//!
//! Texels are 15-bit colors shifted to 8 bits with alpha 0 only for the
//! value 0 (`PSXTexture.ConvertABGR16toABGR32`; the STP bit is ignored).
//!
//! The blur texture (a 15-bit page in the frame buffer area) is the screen
//! the scene left; those batches carry `screen` and the host samples its
//! own frame (`SFXMesh.GetTexture`, `PSXTextureMgr.blurTexture`).
//!
//! A `DR_MOVE` from the display area is a screen shot (`SFXScreenShot`,
//! Slow and Silent Voice): a `capture` batch, and the later batches that
//! use page 7 or 9 sample it. The packets between a `DR_AREA` begin and end
//! are drawn into a 512 x 256 texture (VRAM from 384, 256) that every later
//! 15-bit page reads (`generated_target` and `generated`).

use std::collections::HashMap;

use crate::plugin::Prim;
use crate::vram::{Rect, Vram};

/// `PSXTextureMgr.GEN_TEXTURE_X`, `GEN_TEXTURE_Y`, `_W` and `_H`: the VRAM
/// area the generated texture stands for.
pub const GEN_X: i32 = 384;
pub const GEN_Y: i32 = 256;
pub const GEN_W: usize = 512;
pub const GEN_H: usize = 256;

/// `SpecialEffect.Slow`, `Silent_Voice`, `Devour` and `Stop`.
const SLOW: i32 = 126;
const SILENT_VOICE: i32 = 395;
const DEVOUR: i32 = 274;
const STOP: i32 = 149;

/// Where a textured mesh's texels come from (`SFXMesh.GetTexture`).
enum Source {
    Vram,
    Screen,
    Devour,
    /// The last screen shot, read this many texels further right.
    Shot(f32),
    /// The background capture (`PSXTextureMgr.bgTexture`, host command 120).
    Background,
    /// The generated texture, its page this far right of its left edge.
    Generated(f32),
}

/// `SFX.playParam`: per effect, bits 0-1 the subtractive pass's place
/// (`S1`..`S3`, 0 for the default 3), bits 2-3 the color intensity (`C1`..
/// `C3`, 0 for the default x2), bit 28 additive before opaque (`A1`), bit
/// 29 the color threshold (`O1`).
pub const PLAY_PARAM: &[i32] = &include!("play_param.in");

/// `SFXScreenShot.slowClockUV`: the texel of each vertex of Slow's clock
/// sprite (48 quads), which Memoria puts back over the port's purple glow.
const SLOW_CLOCK_UV: &[[f32; 2]] = &include!("slow_clock_uv.in");
/// The clock's texture: `PSXTextureMgr.GetTexture(1, 1, 8, 247, 0)`, an
/// 8-bit page at (512, 256) with its CLUT at (0, 247).
const SLOW_CLOCK_TEXTURE: (u16, u16) = (0x98, 247 << 6);

/// How a batch writes its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Blend {
    /// Texels under alpha 0.5 are cut out, the rest replace the pixel.
    Opaque,
    /// `Blend SrcAlpha One`.
    Add,
    /// `Blend One One`, `BlendOp RevSub`: the pixel less the color.
    Subtract,
}

/// One corner: PSX screen position (320 x 240, center 160, 120), texel
/// in the 256 x 256 page, and the color the texel is multiplied by (or the
/// color itself without a texture), 1 = unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corner {
    pub xy: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 3],
}

/// Triangles drawn the same way.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    pub blend: Blend,
    /// Index into [`RenderList::pages`].
    pub page: Option<usize>,
    /// The texture is the screen as the scene left it (`PSXTextureMgr`'s
    /// blur texture, `SFXMesh.GetTexture`): a 15-bit page in the frame
    /// buffer area (page x below 6). Corner texels are PSX screen pixels
    /// (320 x 240), and `page` is `None`.
    pub screen: bool,
    /// No triangles: take the screen as it is now, and sample it from here
    /// on (`SFXScreenShot`, the `DR_MOVE` from the display area of Slow and
    /// Silent Voice; the texels of the batches after it are PSX pixels of
    /// that capture).
    pub capture: bool,
    /// No triangles: `DR_AREA` begin. The generated texture (512 x 256,
    /// the VRAM at (384, 256)) is created, black, if the effect has none yet
    /// (`SFXRenderTextureBegin`).
    pub generated_begin: bool,
    /// Drawn into the generated texture, with its texels (`xy` is a pixel
    /// of it), not onto the screen.
    pub generated_target: bool,
    /// The texture is the generated one (`SFXMesh.GetTexture` after a
    /// `DR_AREA` pair): a 15-bit page, `page` is `None` and the texels are
    /// the page's shifted by its place (`tx * 64 - 384`) in a 512 x 256
    /// texture.
    pub generated: bool,
    /// The texture is the background capture (`SFXMesh.GetTexture`, `bgKey`):
    /// a 256 x 256 picture of the battle ground, `page` is `None` and the
    /// texels are the page's own.
    pub background: bool,
    pub triangles: Vec<[Corner; 3]>,
}

impl Batch {
    fn marker(capture: bool, generated_begin: bool) -> Self {
        Self {
            blend: Blend::Opaque,
            page: None,
            screen: false,
            capture,
            generated_begin,
            generated_target: false,
            generated: false,
            background: false,
            triangles: Vec::new(),
        }
    }
}

/// A texture page as 256 x 256 RGBA8, rows top-down (row `v`).
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderList {
    pub batches: Vec<Batch>,
    pub pages: Vec<Page>,
    /// The seven integers of a background capture the plugin asked for this
    /// frame (host command 120, `PSXTextureMgr.bgParam`): texture place,
    /// screen size and the camera shift. Set by the player.
    pub background_capture: Option<[i32; 7]>,
}

impl RenderList {
    pub fn triangles(&self) -> usize {
        self.batches.iter().map(|batch| batch.triangles.len()).sum()
    }
}

/// `SFXRender` mesh lists, in the order `PushCommandBufferOpa`, `..Add`
/// and `..Sub` walk them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum List {
    Opa,
    TexOpa,
    LineOpa,
    Add,
    /// `meshTexAddPS`: point-filtered (and unfiltered) textures.
    TexAdd,
    /// `meshTexAddBL`: bilinear ones (sprites, the blurred packets).
    TexAddSmooth,
    LineAdd,
    Sub,
    TexSub,
    LineSub,
}

impl List {
    fn blend(self) -> Blend {
        match self {
            Self::Opa | Self::TexOpa | Self::LineOpa => Blend::Opaque,
            Self::Add | Self::TexAdd | Self::TexAddSmooth | Self::LineAdd => Blend::Add,
            _ => Blend::Subtract,
        }
    }
}

/// A mesh being filled: its list, texture key and triangles.
struct Mesh {
    list: List,
    texture: Option<(u16, u16)>,
    /// `SFXKey` filter bits: 0, point (1) or bilinear (2).
    filter: u8,
    /// Drawn between a `DR_AREA` begin and end.
    offscreen: bool,
    /// Slow's clock mesh (`SFXScreenShot.IsSpecialSlowTexture`): the
    /// subtractive flat mesh of key 0x01000000, drawn as a textured one.
    clock: bool,
    triangles: Vec<[Corner; 3]>,
}

struct Builder<'a> {
    vram: &'a mut Vram,
    /// `SFXKey.currentTexPage` (from `DR_TPAGE`).
    tpage: u16,
    offset: [i32; 2],
    /// Inside a `DR_AREA` render-to-texture pair.
    offscreen: bool,
    /// The effect (`SFX.currentEffectID`).
    effect: i32,
    /// `PSXTextureMgr.isCreateGenTexture`.
    generated_created: bool,
    /// `PSXTextureMgr.bgKey` (see [`Vram::background`]).
    background: Option<u16>,
    /// Where the last screen shot (`DR_MOVE`) starts on the PSX screen.
    shot: Option<[i32; 2]>,
    /// Color intensity (`ColorData`).
    intensity: f32,
    sub_order: i32,
    add_first: bool,
    /// Vertices the clock mesh has so far (its `VbOffset`).
    clock_vertices: usize,
    meshes: Vec<Mesh>,
    /// The filter of the packet being added.
    filter: u8,
    out: RenderList,
    pages: HashMap<(u16, u16), usize>,
}

fn xy(word: u32) -> [i32; 2] {
    [i32::from(word as i16), i32::from((word >> 16) as i16)]
}

fn uv(word: u32) -> [f32; 2] {
    [(word & 0xFF) as f32, ((word >> 8) & 0xFF) as f32]
}

fn rgb(word: u32) -> [u8; 3] {
    [word as u8, (word >> 8) as u8, (word >> 16) as u8]
}

/// `SFXMesh.PolyBft4` / `PolyBgt4`: the plugin's blurred packets (0x44 and
/// 0x48) shift u by 128 on any page but the first.
fn blur_shift(code: u8, tpage: u16) -> f32 {
    if matches!(code & 0xFC, 0x44 | 0x48) && tpage & 0x1F != 0 {
        128.0
    } else {
        0.0
    }
}

/// `SFXMesh.AbrAlphaData`.
const ABR_ALPHA: [u8; 4] = [63, 127, 127, 31];

impl Builder<'_> {
    /// The mesh list and vertex alpha of a packet (`SFXKey.GetABRTex`,
    /// `GetCurrentABR`, `SFXMesh.Setup` and `GetShadeAlpha`).
    fn kind(&self, code: u8, tpage: u16, textured: bool, line: bool) -> (List, f32) {
        let abr = usize::from((tpage >> 5) & 3);
        let semi = code & 2 != 0;
        let base = if semi { ABR_ALPHA[abr] } else { 128 };
        let alpha = if textured && code & 1 == 0 {
            (u32::from(base) * 2).min(255) as u8
        } else {
            base
        };
        let list = match (semi, abr == 2, textured, line) {
            (false, _, _, true) => List::LineOpa,
            (false, _, true, _) => List::TexOpa,
            (false, _, false, _) => List::Opa,
            (true, false, _, true) => List::LineAdd,
            (true, false, true, _) => List::TexAdd,
            (true, false, false, _) => List::Add,
            (true, true, _, true) => List::LineSub,
            (true, true, true, _) => List::TexSub,
            (true, true, false, _) => List::Sub,
        };
        (list, f32::from(alpha) / 255.0)
    }

    fn corner(&self, xy: [i32; 2], uv: [f32; 2], color: [u8; 3], scale: f32) -> Corner {
        Corner {
            xy: [
                (xy[0] + self.offset[0]) as f32,
                (xy[1] + self.offset[1]) as f32,
            ],
            uv,
            color: color.map(|c| f32::from(c) / 255.0 * scale),
        }
    }

    fn add(&mut self, list: List, texture: Option<(u16, u16)>, triangles: &[[Corner; 3]]) {
        // The key holds the page, depth and CLUT, not the blend mode.
        let texture = texture.map(|(tpage, clut)| (tpage & 0x19F, clut));
        let filter = self.filter;
        let list = if list == List::TexAdd && filter == 2 {
            List::TexAddSmooth
        } else {
            list
        };
        // Mesh key 0x01000000: subtractive, untextured, page 0.
        let clock = self.effect == SLOW
            && list == List::Sub
            && texture.is_none()
            && self.tpage & 0x19F == 0;
        let mut triangles = triangles.to_vec();
        if clock {
            self.clock_uv(&mut triangles);
        }
        let offscreen = self.offscreen;
        if let Some(mesh) = self.meshes.iter_mut().find(|mesh| {
            mesh.list == list
                && mesh.texture == texture
                && mesh.filter == filter
                && mesh.offscreen == offscreen
        }) {
            mesh.triangles.extend_from_slice(&triangles);
            return;
        }
        self.meshes.push(Mesh {
            list,
            texture,
            filter,
            offscreen,
            clock,
            triangles,
        });
    }

    /// `SFXMesh.End` for Slow's clock mesh: the vertices, in the order the
    /// packets added them, take the clock's texels (a quad's are corners 0,
    /// 1, 2, 3 and its triangles (0, 1, 2) and (2, 1, 3)).
    fn clock_uv(&mut self, triangles: &mut [[Corner; 3]]) {
        let texel = |at: usize| SLOW_CLOCK_UV.get(at).copied().unwrap_or([0.0; 2]);
        let first = self.clock_vertices;
        match triangles {
            [a] => {
                for (i, corner) in a.iter_mut().enumerate() {
                    corner.uv = texel(first + i);
                }
                self.clock_vertices += 3;
            }
            [a, b] => {
                for (i, corner) in a.iter_mut().enumerate() {
                    corner.uv = texel(first + i);
                }
                b[0] = a[2];
                b[1] = a[1];
                b[2].uv = texel(first + 3);
                self.clock_vertices += 4;
            }
            _ => {}
        }
    }

    fn quad(&mut self, list: List, texture: Option<(u16, u16)>, c: [Corner; 4]) {
        // `SFXMesh.PolyFt4`: (0, 1, 2) and (2, 1, 3).
        self.add(list, texture, &[[c[0], c[1], c[2]], [c[2], c[1], c[3]]]);
    }

    /// One packet (`SFXRender.Add`).
    fn packet(&mut self, words: &[u32]) {
        let Some(&first) = words.get(1) else {
            return;
        };
        let code = (first >> 24) as u8;
        let w = |i: usize| words.get(i + 1).copied().unwrap_or(0);
        // `SFXRender.Add`: 0x4C is point filtered; sprites and the blurred
        // 0x44 and 0x48 bilinear.
        self.filter = match code & 0xFC {
            0x4C => 1,
            0x44 | 0x48 | 0x64 | 0x74 | 0x7C => 2,
            _ => 0,
        };
        match code & 0xFC {
            // POLY_F3 / POLY_G3.
            0x20 | 0x30 => {
                let gouraud = code & 0x10 != 0;
                let (list, alpha) = self.kind(code, self.tpage, false, false);
                let scale = alpha * self.intensity;
                let corners: Vec<Corner> = (0..3)
                    .map(|i| {
                        let (color, at) = if gouraud {
                            (rgb(w(2 * i)), xy(w(2 * i + 1)))
                        } else {
                            (rgb(w(0)), xy(w(1 + i)))
                        };
                        self.corner(at, [0.0; 2], color, scale)
                    })
                    .collect();
                self.add(list, None, &[[corners[0], corners[1], corners[2]]]);
            }
            // POLY_F4 / POLY_G4.
            0x28 | 0x38 => {
                let gouraud = code & 0x10 != 0;
                let (list, alpha) = self.kind(code, self.tpage, false, false);
                let scale = alpha * self.intensity;
                let c: Vec<Corner> = (0..4)
                    .map(|i| {
                        let (color, at) = if gouraud {
                            (rgb(w(2 * i)), xy(w(2 * i + 1)))
                        } else {
                            (rgb(w(0)), xy(w(1 + i)))
                        };
                        self.corner(at, [0.0; 2], color, scale)
                    })
                    .collect();
                self.quad(list, None, [c[0], c[1], c[2], c[3]]);
            }
            // POLY_FT3.
            0x24 => {
                let (clut, tpage) = ((w(2) >> 16) as u16, (w(4) >> 16) as u16);
                let (list, alpha) = self.kind(code, tpage, true, false);
                let scale = alpha * self.intensity;
                let color = rgb(w(0));
                let c: Vec<Corner> = (0..3)
                    .map(|i| self.corner(xy(w(1 + 2 * i)), uv(w(2 + 2 * i)), color, scale))
                    .collect();
                self.add(list, Some((tpage, clut)), &[[c[0], c[1], c[2]]]);
            }
            // POLY_FT4, and the plugin's 0x44 (blurred) and 0x4C (point
            // filtered) variants.
            0x2C | 0x44 | 0x4C => {
                let (clut, tpage) = ((w(2) >> 16) as u16, (w(4) >> 16) as u16);
                let (list, alpha) = self.kind(code, tpage, true, false);
                let scale = alpha * self.intensity;
                let color = rgb(w(0));
                let shift = blur_shift(code, tpage);
                let c: Vec<Corner> = (0..4)
                    .map(|i| {
                        let mut texel = uv(w(2 + 2 * i));
                        texel[0] += shift;
                        self.corner(xy(w(1 + 2 * i)), texel, color, scale)
                    })
                    .collect();
                self.quad(list, Some((tpage, clut)), [c[0], c[1], c[2], c[3]]);
            }
            // POLY_GT3.
            0x34 => {
                let (clut, tpage) = ((w(2) >> 16) as u16, (w(5) >> 16) as u16);
                let (list, alpha) = self.kind(code, tpage, true, false);
                let scale = alpha * self.intensity;
                let c: Vec<Corner> = (0..3)
                    .map(|i| self.corner(xy(w(1 + 3 * i)), uv(w(2 + 3 * i)), rgb(w(3 * i)), scale))
                    .collect();
                self.add(list, Some((tpage, clut)), &[[c[0], c[1], c[2]]]);
            }
            // POLY_GT4, and the plugin's blurred 0x48.
            0x3C | 0x48 => {
                let (clut, tpage) = ((w(2) >> 16) as u16, (w(5) >> 16) as u16);
                let (list, alpha) = self.kind(code, tpage, true, false);
                let scale = alpha * self.intensity;
                let shift = blur_shift(code, tpage);
                let c: Vec<Corner> = (0..4)
                    .map(|i| {
                        let mut texel = uv(w(2 + 3 * i));
                        texel[0] += shift;
                        self.corner(xy(w(1 + 3 * i)), texel, rgb(w(3 * i)), scale)
                    })
                    .collect();
                self.quad(list, Some((tpage, clut)), [c[0], c[1], c[2], c[3]]);
            }
            // LINE_F2 / LINE_G2, drawn one pixel wide.
            0x40 | 0x50 => {
                let gouraud = code & 0x10 != 0;
                let (list, alpha) = self.kind(code, self.tpage, false, true);
                let scale = alpha * self.intensity;
                let (c0, p0, c1, p1) = if gouraud {
                    (rgb(w(0)), xy(w(1)), rgb(w(2)), xy(w(3)))
                } else {
                    (rgb(w(0)), xy(w(1)), rgb(w(0)), xy(w(2)))
                };
                let (a, b) = (
                    self.corner(p0, [0.0; 2], c0, scale),
                    self.corner(p1, [0.0; 2], c1, scale),
                );
                let (dx, dy) = (b.xy[0] - a.xy[0], b.xy[1] - a.xy[1]);
                let length = (dx * dx + dy * dy).sqrt().max(1e-3);
                let n = [-dy / length * 0.5, dx / length * 0.5];
                let shift = |c: Corner, s: f32| Corner {
                    xy: [c.xy[0] + n[0] * s, c.xy[1] + n[1] * s],
                    ..c
                };
                self.quad(
                    list,
                    None,
                    [shift(a, 1.0), shift(b, 1.0), shift(a, -1.0), shift(b, -1.0)],
                );
            }
            // TILE, TILE_1, TILE_8, TILE_16.
            0x60 | 0x68 | 0x70 | 0x78 => {
                let size = match code & 0xFC {
                    0x60 => xy(w(2)),
                    0x68 => [1, 1],
                    0x70 => [8, 8],
                    _ => [16, 16],
                };
                let (list, alpha) = self.kind(code, self.tpage, false, false);
                let scale = alpha * self.intensity;
                let at = xy(w(1));
                let color = rgb(w(0));
                let c = |dx: i32, dy: i32| {
                    self.corner([at[0] + dx, at[1] + dy], [0.0; 2], color, scale)
                };
                let corners = [c(0, 0), c(size[0], 0), c(0, size[1]), c(size[0], size[1])];
                // `SFXMesh.Tile`/`Sprite`: (0, 1, 2) and (1, 3, 2).
                self.add(
                    list,
                    None,
                    &[
                        [corners[0], corners[1], corners[2]],
                        [corners[1], corners[3], corners[2]],
                    ],
                );
            }
            // SPRT, SPRT_8, SPRT_16: the current texture page.
            0x64 | 0x74 | 0x7C => {
                let size = match code & 0xFC {
                    0x64 => xy(w(3)),
                    0x74 => [8, 8],
                    _ => [16, 16],
                };
                let clut = (w(2) >> 16) as u16;
                let (list, alpha) = self.kind(code, self.tpage, true, false);
                let scale = alpha * self.intensity;
                let at = xy(w(1));
                let base = uv(w(2));
                let color = rgb(w(0));
                // `SFXMesh.Sprite`: texels run to `w - 1`.
                let (du, dv) = ((size[0] - 1) as f32, (size[1] - 1) as f32);
                let c = |dx: i32, dy: i32, u: f32, v: f32| {
                    self.corner(
                        [at[0] + dx, at[1] + dy],
                        [base[0] + u, base[1] + v],
                        color,
                        scale,
                    )
                };
                let corners = [
                    c(0, 0, 0.0, 0.0),
                    c(size[0], 0, du, 0.0),
                    c(0, size[1], 0.0, dv),
                    c(size[0], size[1], du, dv),
                ];
                self.add(
                    list,
                    Some((self.tpage, clut)),
                    &[
                        [corners[0], corners[1], corners[2]],
                        [corners[1], corners[3], corners[2]],
                    ],
                );
            }
            _ => match code {
                // DR_TPAGE.
                0xE1 => self.tpage = first as u16,
                // DR_AREA (`SFXRender.DR_AREA`): draws between a begin and an
                // end go to the generated texture, whose origin is VRAM
                // (384, 256).
                0xE4 => {
                    self.flush();
                    let begin = w(1) >> 16 != 0;
                    if begin {
                        self.offset[0] -= GEN_X;
                        self.offset[1] -= GEN_Y;
                        self.out.batches.push(Batch::marker(false, true));
                    } else if self.offscreen {
                        // `SFXRenderTextureEnd`.
                        self.generated_created = true;
                    }
                    self.offscreen = begin;
                }
                // DR_OFFSET.
                0xE5 => {
                    let value = w(1);
                    self.offset = [i32::from(value as u16), i32::from((value >> 16) as u16)];
                }
                // DR_MOVE: a VRAM copy, or (from the frame buffer) a screen
                // shot.
                0xE7 => {
                    self.flush();
                    let (rx, ry) = (i32::from((w(1) >> 16) as u16), i32::from(w(1) as i16));
                    let (rw, rh) = ((w(2) >> 16) as i16, w(2) as i16);
                    let (x, y) = ((w(3) >> 16) as i16, w(3) as i16);
                    if rx < 320 && ry < 240 {
                        // `SFXRender.DR_MOVE`: a `SFXScreenShot` at (rx, ry);
                        // Slow's sits 128 rows higher.
                        let ry = if self.effect == SLOW { ry - 128 } else { ry };
                        self.shot = Some([rx, ry]);
                        self.out.batches.push(Batch::marker(true, false));
                    } else {
                        self.vram.move_image(
                            Rect {
                                x: rx as i16,
                                y: ry as i16,
                                w: rw,
                                h: rh,
                            },
                            x,
                            y,
                        );
                        self.pages.clear();
                    }
                }
                _ => {}
            },
        }
    }

    /// `SFXRender.PushCommandBuffer`: the meshes so far in pass order.
    fn flush(&mut self) {
        let mut meshes = std::mem::take(&mut self.meshes);
        let pass = |list: List| match list.blend() {
            Blend::Opaque => usize::from(self.add_first),
            Blend::Add => usize::from(!self.add_first),
            Blend::Subtract => 0,
        };
        // Opa and Add keep their relative order (`addOrder`); Sub goes in
        // slot `subOrder` (0 first, 1 between, 2 last).
        let order = |list: List| -> (usize, List) {
            let slot = match list.blend() {
                Blend::Subtract => self.sub_order as usize * 2,
                _ => pass(list) * 2 + 1,
            };
            (slot, list)
        };
        meshes.sort_by_key(|mesh| order(mesh.list));
        for mut mesh in meshes {
            let mut repeat = 1;
            // `SFXMesh.End`: the clock mesh becomes `SFX_ADD_GT` over the
            // clock texture, with white vertex colors.
            if mesh.clock {
                mesh.list = List::TexAdd;
                mesh.texture = Some(SLOW_CLOCK_TEXTURE);
                for corner in mesh.triangles.iter_mut().flatten() {
                    corner.color = [self.intensity; 3];
                }
            }
            // `SFXMesh.Setup` and `Render`: Stop's black sprite (key
            // 0xD78000: additive, 15-bit page 7 of row 1) is `SFX_SUB_GT`,
            // drawn twice.
            if self.effect == STOP
                && mesh
                    .texture
                    .is_some_and(|(tpage, clut)| tpage == 0x117 && clut & 0x7FFF == 0)
            {
                if mesh.list == List::TexAdd && mesh.filter == 0 {
                    mesh.list = List::TexSub;
                }
                repeat = 2;
            }
            let mut screen = false;
            let mut generated = false;
            let mut background = false;
            let mut triangles = mesh.triangles;
            // `SFXMesh.GetTexture`: what a 15-bit page (mode 2) really is.
            let source = mesh
                .texture
                .map_or(Source::Vram, |(tpage, clut)| self.source(tpage, clut));
            match source {
                Source::Vram => {}
                // The background capture, a 256 x 256 texture read by the
                // page's own texels.
                Source::Background => background = true,
                // Left of page 6: the screen the scene left
                // (`SFXKey.IsBlurTexture`); the packets' texels address the
                // captured screen, not VRAM.
                Source::Screen => screen = true,
                // Devour's blur texture, a 480 x 360 view of the screen
                // that starts 176 texels in (`_constTexParam` 176, 0, 480,
                // 360), in PSX screen pixels.
                Source::Devour => {
                    screen = true;
                    for corner in triangles.iter_mut().flatten() {
                        corner.uv = [(corner.uv[0] + 176.0) / 1.5, corner.uv[1] / 1.5];
                    }
                }
                // Slow's page 7 and Silent Voice's pages 7 and 9 (texture
                // keys 0x578000 and 0x598000) are the last screen shot;
                // page 9 reads 128 texels further right.
                Source::Shot(shift) => {
                    // No shot yet: the texture is missing and nothing draws.
                    let Some(origin) = self.shot else {
                        continue;
                    };
                    screen = true;
                    for corner in triangles.iter_mut().flatten() {
                        corner.uv[0] += shift + origin[0] as f32;
                        corner.uv[1] += origin[1] as f32;
                    }
                }
                // After a `DR_AREA` pair: the generated texture, which
                // stands for VRAM from (384, 256).
                Source::Generated(shift) => {
                    generated = true;
                    for corner in triangles.iter_mut().flatten() {
                        corner.uv[0] += shift;
                    }
                }
            }
            let page = mesh
                .texture
                .filter(|_| !screen && !generated && !background)
                .map(|key| self.page(key));
            for _ in 0..repeat {
                self.out.batches.push(Batch {
                    blend: mesh.list.blend(),
                    page,
                    screen,
                    capture: false,
                    generated_begin: false,
                    generated_target: mesh.offscreen,
                    generated,
                    background,
                    triangles: triangles.clone(),
                });
            }
        }
        self.clock_vertices = 0;
    }

    /// `SFXMesh.GetTexture`, in its order, for a textured mesh.
    fn source(&self, tpage: u16, clut: u16) -> Source {
        if (tpage >> 7) & 3 != 2 {
            return Source::Vram;
        }
        let (tx, ty) = (tpage & 0xF, (tpage >> 4) & 1);
        if tx < 6 {
            return Source::Screen;
        }
        // `(_key & 0x1F0000) == bgKey`, after the blur test.
        if self.background == Some(tx | (ty << 4)) {
            return Source::Background;
        }
        // The texture key without the HAS_TEXTURE bit.
        let low = clut & 0x7FFF;
        match (self.effect, tx, ty, low) {
            (DEVOUR, 7, 1, 0x7FFF) => Source::Devour,
            (SLOW | SILENT_VOICE, 7, 1, 0) => Source::Shot(0.0),
            (SILENT_VOICE, 9, 1, 0) => Source::Shot(128.0),
            // Stop's black sprite reads its page.
            (STOP, 7, 1, 0) => Source::Vram,
            _ if self.generated_created => Source::Generated(f32::from(tx) * 64.0 - GEN_X as f32),
            _ => Source::Vram,
        }
    }

    fn page(&mut self, (tpage, clut): (u16, u16)) -> usize {
        if let Some(&index) = self.pages.get(&(tpage, clut)) {
            return index;
        }
        let rgba = page_rgba(self.vram, tpage, clut);
        self.out.pages.push(Page { rgba });
        let index = self.out.pages.len() - 1;
        self.pages.insert((tpage, clut), index);
        index
    }
}

/// `PSXTexture.CreateBufferColor32` for a 256 x 256 page.
pub fn page_rgba(vram: &Vram, tpage: u16, clut: u16) -> Vec<u8> {
    let (px, py) = (
        i32::from(tpage & 0xF) * 64,
        i32::from((tpage >> 4) & 1) * 256,
    );
    let depth = (tpage >> 7) & 3;
    let (cx, cy) = (i32::from(clut & 0x3F) * 16, i32::from(clut >> 6));
    let mut out = Vec::with_capacity(256 * 256 * 4);
    for y in 0..256 {
        for x in 0..256 {
            let value = match depth {
                0 => {
                    let word = vram.pixel(px + x / 4, py + y);
                    vram.pixel(cx + i32::from((word >> ((x % 4) * 4)) & 0xF), cy)
                }
                1 => {
                    let word = vram.pixel(px + x / 2, py + y);
                    vram.pixel(cx + i32::from((word >> ((x % 2) * 8)) & 0xFF), cy)
                }
                _ => vram.pixel(px + x, py + y),
            };
            out.extend_from_slice(&[
                ((value & 0x1F) << 3) as u8,
                ((value >> 5 & 0x1F) << 3) as u8,
                ((value >> 10 & 0x1F) << 3) as u8,
                if value == 0 { 0 } else { 255 },
            ]);
        }
    }
    out
}

/// A frame's packets as the PC game draws them. `vram` is the effect's
/// VRAM after the frame (the packets' `DR_MOVE` copies are applied to it).
pub fn render_list(effect: i32, prims: &[Prim], vram: &mut Vram) -> RenderList {
    let param = usize::try_from(effect)
        .ok()
        .and_then(|id| PLAY_PARAM.get(id))
        .copied()
        .unwrap_or(0);
    // `SFX.Play`: `subOrder` 0..2 (default 2), `colIntensity` index into
    // (1, 1.5, 2) (default 2).
    let sub_order = match param & 3 {
        0 => 2,
        order => order - 1,
    };
    let intensity = match (param >> 2) & 3 {
        1 => 1.0,
        2 => 1.5,
        _ => 2.0,
    };
    let generated_created = vram.generated_created;
    let background = vram.background;
    let mut builder = Builder {
        vram,
        tpage: 0,
        offset: [0, 0],
        offscreen: false,
        effect,
        generated_created,
        background,
        shot: None,
        intensity,
        sub_order,
        add_first: (param >> 28) & 1 != 0,
        clock_vertices: 0,
        meshes: Vec::new(),
        filter: 0,
        out: RenderList::default(),
        pages: HashMap::new(),
    };
    for (_, words) in prims {
        builder.packet(words);
    }
    builder.flush();
    builder.vram.generated_created = builder.generated_created;
    builder.out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A POLY_FT4 packet: code, color, four corners (x, y, u, v), CLUT, tpage.
    fn quad(code: u8, tpage: u16, corners: [[u8; 4]; 4]) -> (i32, Vec<u32>) {
        let mut words = vec![0, u32::from(code) << 24 | 0x80_8080];
        for (i, [x, y, u, v]) in corners.into_iter().enumerate() {
            words.push(u32::from(y) << 16 | u32::from(x));
            let high = match i {
                0 => 0,
                1 => u32::from(tpage) << 16,
                _ => 0,
            };
            words.push(high | u32::from(v) << 8 | u32::from(u));
        }
        (0, words)
    }

    #[test]
    fn a_15_bit_page_left_of_page_6_is_the_screen() {
        let corners = [[0, 0, 0, 0], [8, 0, 8, 0], [0, 8, 0, 8], [8, 8, 8, 8]];
        let mut vram = Vram::default();
        // 15-bit (mode 2) pages 1 and 6.
        let screen = render_list(0, &[quad(0x2C, 0x101, corners)], &mut vram);
        assert!(screen.batches[0].screen && screen.batches[0].page.is_none());
        let page = render_list(0, &[quad(0x2C, 0x106, corners)], &mut vram);
        assert!(!page.batches[0].screen && page.batches[0].page.is_some());
    }

    #[test]
    fn the_plugins_blurred_packets_shift_u_on_later_pages() {
        let corners = [[0, 0, 0, 0], [8, 0, 8, 0], [0, 8, 0, 8], [8, 8, 8, 8]];
        let mut vram = Vram::default();
        let list = render_list(0, &[quad(0x44, 0x101, corners)], &mut vram);
        assert_eq!(list.batches[0].triangles[0][0].uv, [128.0, 0.0]);
        let list = render_list(0, &[quad(0x44, 0x100, corners)], &mut vram);
        assert_eq!(list.batches[0].triangles[0][0].uv, [0.0, 0.0]);
    }

    /// A `DR_MOVE` packet from (rx, ry), 8 x 8, to (0, 0).
    fn screen_move(rx: u16, ry: u16) -> (i32, Vec<u32>) {
        (
            0,
            vec![
                0,
                0xE700_0000,
                u32::from(rx) << 16 | u32::from(ry),
                8 << 16 | 8,
                0,
            ],
        )
    }

    #[test]
    fn a_screen_shot_is_taken_where_slow_and_silent_voice_ask() {
        let corners = [[0, 0, 0, 0], [8, 0, 8, 0], [0, 8, 0, 8], [8, 8, 8, 8]];
        let mut vram = Vram::default();
        // Page 7, y 1, 15-bit: Slow's screen shot (its DR_MOVE sits 128
        // rows up), Silent Voice's page 9 reads 128 texels further right.
        let packets = [
            screen_move(20, 140),
            quad(0x2C, 0x117, corners),
            quad(0x2C, 0x119, corners),
        ];
        let slow = render_list(126, &packets, &mut vram);
        assert!(slow.batches[0].capture);
        assert!(slow.batches[1].screen && !slow.batches[2].screen);
        assert_eq!(slow.batches[1].triangles[0][0].uv, [20.0, 12.0]);
        let voice = render_list(395, &packets, &mut vram);
        assert_eq!(voice.batches.len(), 3);
        assert_eq!(voice.batches[1].triangles[0][0].uv, [20.0, 140.0]);
        assert_eq!(voice.batches[2].triangles[0][0].uv, [148.0, 140.0]);
        // Any other effect draws the page from VRAM.
        let other = render_list(0, &packets, &mut vram);
        assert!(other.batches[1..].iter().all(|batch| !batch.screen));
    }

    /// A `DR_OFFSET` or `DR_AREA` packet.
    fn setting(code: u32, value: u32) -> (i32, Vec<u32>) {
        (0, vec![0, code << 24, value])
    }

    #[test]
    fn a_dr_area_pair_makes_the_generated_texture_later_15_bit_pages_read() {
        let corners = [[0, 0, 0, 0], [8, 0, 8, 0], [0, 8, 0, 8], [8, 8, 8, 8]];
        let mut vram = Vram::default();
        let packets = [
            // Before the pair: page 7 is VRAM.
            quad(0x2C, 0x117, corners),
            setting(0xE5, 384 << 16 | 512),
            setting(0xE4, 512 << 16 | 128),
            quad(0x2C, 0x113, corners),
            setting(0xE5, 0),
            setting(0xE4, 0),
            quad(0x2C, 0x117, corners),
            // A 4-bit page is always VRAM.
            quad(0x2C, 0x017, corners),
        ];
        let list = render_list(0, &packets, &mut vram);
        let kinds: Vec<_> = list
            .batches
            .iter()
            .map(|batch| {
                (
                    batch.generated_begin,
                    batch.generated_target,
                    batch.generated,
                )
            })
            .collect();
        assert_eq!(
            kinds,
            [
                (false, false, false),
                (true, false, false),
                (false, true, false),
                (false, false, true),
                (false, false, false),
            ]
        );
        // Drawn at the VRAM place less (384, 256), and read 64 texels in
        // (page 7 is at 448).
        assert_eq!(list.batches[2].triangles[0][0].xy, [128.0, 128.0]);
        assert_eq!(list.batches[3].triangles[0][0].uv, [64.0, 0.0]);
        // It stays created for the effect's next frames.
        assert!(vram.generated_created);
        let next = render_list(0, &[quad(0x2C, 0x117, corners)], &mut vram);
        assert!(next.batches[0].generated);
    }

    /// A POLY_G4 packet (code 0x3A, semi-transparent) at the given corners.
    fn gouraud_quad(corners: [[u16; 2]; 4]) -> (i32, Vec<u32>) {
        let mut words = vec![0];
        for (i, [x, y]) in corners.into_iter().enumerate() {
            words.push(if i == 0 { 0x3A00_0000 } else { 0 } | 0x80_8080);
            words.push(u32::from(y) << 16 | u32::from(x));
        }
        (0, words)
    }

    #[test]
    fn a_background_capture_is_read_by_its_page() {
        let corners = [[0, 0, 0, 0], [8, 0, 8, 0], [0, 8, 0, 8], [8, 8, 8, 8]];
        let mut vram = Vram::default();
        // Page 7 of row 1 (448, 256) is VRAM until the effect captures.
        let before = render_list(0, &[quad(0x2C, 0x117, corners)], &mut vram);
        assert!(!before.batches[0].background && before.batches[0].page.is_some());
        vram.background = Some(7 | 1 << 4);
        let after = render_list(0, &[quad(0x2C, 0x117, corners)], &mut vram);
        assert!(after.batches[0].background && after.batches[0].page.is_none());
        // Another page, and the blur pages, are unchanged.
        let other = render_list(0, &[quad(0x2C, 0x118, corners)], &mut vram);
        assert!(!other.batches[0].background);
        vram.background = Some(2);
        let blur = render_list(0, &[quad(0x2C, 0x102, corners)], &mut vram);
        assert!(blur.batches[0].screen && !blur.batches[0].background);
    }

    #[test]
    fn slows_subtractive_glow_is_the_clock_and_stops_black_sprite_is_subtractive() {
        let mut vram = Vram::default();
        // Slow: a subtractive untextured quad (tpage 0x240) becomes an
        // additive textured one with the clock's texels.
        let packets = [
            (0, vec![0, 0xE100_0240]),
            gouraud_quad([[0, 0], [8, 0], [0, 8], [8, 8]]),
        ];
        let slow = render_list(126, &packets, &mut vram);
        assert_eq!(slow.batches.len(), 1);
        let batch = &slow.batches[0];
        assert!(batch.blend == Blend::Add && batch.page.is_some());
        assert_eq!(batch.triangles[0][0].uv, SLOW_CLOCK_UV[0]);
        assert_eq!(batch.triangles[0][2].uv, SLOW_CLOCK_UV[2]);
        assert_eq!(batch.triangles[1][2].uv, SLOW_CLOCK_UV[3]);
        let other = render_list(0, &packets, &mut vram);
        assert!(other.batches[0].blend == Blend::Subtract && other.batches[0].page.is_none());
        // Stop: key 0xD78000 (additive, page 7 of row 1, 15 bits) is
        // subtractive and drawn twice.
        let corners = [[0, 0, 0, 0], [8, 0, 8, 0], [0, 8, 0, 8], [8, 8, 8, 8]];
        let stop = render_list(149, &[quad(0x2E, 0x117, corners)], &mut vram);
        assert_eq!(stop.batches.len(), 2);
        assert!(
            stop.batches
                .iter()
                .all(|batch| batch.blend == Blend::Subtract)
        );
    }
}
