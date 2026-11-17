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
//! Not drawn yet: `DR_MOVE` screen shots into a texture (`SFXScreenShot`,
//! Slow and Silent Voice) and the packets between a `DR_AREA` begin and
//! end, which the game draws into a texture.

use std::collections::HashMap;

use crate::plugin::Prim;
use crate::vram::{Rect, Vram};

/// `SFX.playParam`: per effect, bits 0-1 the subtractive pass's place
/// (`S1`..`S3`, 0 for the default 3), bits 2-3 the color intensity (`C1`..
/// `C3`, 0 for the default x2), bit 28 additive before opaque (`A1`), bit
/// 29 the color threshold (`O1`).
pub const PLAY_PARAM: &[i32] = &include!("play_param.in");

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
    pub triangles: Vec<[Corner; 3]>,
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
    triangles: Vec<[Corner; 3]>,
}

struct Builder<'a> {
    vram: &'a mut Vram,
    /// `SFXKey.currentTexPage` (from `DR_TPAGE`).
    tpage: u16,
    offset: [i32; 2],
    /// Inside a `DR_AREA` render-to-texture pair.
    offscreen: bool,
    /// Color intensity (`ColorData`).
    intensity: f32,
    sub_order: i32,
    add_first: bool,
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
        if self.offscreen {
            return;
        }
        // The key holds the page, depth and CLUT, not the blend mode.
        let texture = texture.map(|(tpage, clut)| (tpage & 0x19F, clut));
        let filter = self.filter;
        let list = if list == List::TexAdd && filter == 2 {
            List::TexAddSmooth
        } else {
            list
        };
        if let Some(mesh) = self
            .meshes
            .iter_mut()
            .find(|mesh| mesh.list == list && mesh.texture == texture && mesh.filter == filter)
        {
            mesh.triangles.extend_from_slice(triangles);
            return;
        }
        self.meshes.push(Mesh {
            list,
            texture,
            filter,
            triangles: triangles.to_vec(),
        });
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
                // DR_AREA: draws between a begin and an end go to a texture.
                0xE4 => {
                    self.flush();
                    self.offscreen = w(1) >> 16 != 0;
                }
                // DR_OFFSET.
                0xE5 => {
                    let value = w(1);
                    self.offset = [i32::from(value as u16), i32::from((value >> 16) as u16)];
                }
                // DR_MOVE: a VRAM copy, or (from the frame buffer) a screen
                // capture, which is not drawn.
                0xE7 => {
                    self.flush();
                    let (rx, ry) = (i32::from((w(1) >> 16) as u16), i32::from(w(1) as i16));
                    let (rw, rh) = ((w(2) >> 16) as i16, w(2) as i16);
                    let (x, y) = ((w(3) >> 16) as i16, w(3) as i16);
                    if !(rx < 320 && ry < 240) {
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
        for mesh in meshes {
            // `SFXMesh.GetTexture`: a 15-bit page (mode 2) left of page 6
            // is the screen (`SFXKey.IsBlurTexture`); the packets' texels
            // address the captured screen, not VRAM.
            let screen = mesh
                .texture
                .is_some_and(|(tpage, _)| (tpage >> 7) & 3 == 2 && tpage & 0xF < 6);
            let page = mesh.texture.filter(|_| !screen).map(|key| self.page(key));
            self.out.batches.push(Batch {
                blend: mesh.list.blend(),
                page,
                screen,
                triangles: mesh.triangles,
            });
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
    let mut builder = Builder {
        vram,
        tpage: 0,
        offset: [0, 0],
        offscreen: false,
        intensity,
        sub_order,
        add_first: (param >> 28) & 1 != 0,
        meshes: Vec::new(),
        filter: 0,
        out: RenderList::default(),
        pages: HashMap::new(),
    };
    for (_, words) in prims {
        builder.packet(words);
    }
    builder.flush();
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
}
