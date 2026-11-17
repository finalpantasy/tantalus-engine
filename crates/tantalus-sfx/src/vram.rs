//! PS1 VRAM: 1024x512 16-bit pixels (`PSXTextureMgr.originalVram`). The
//! effect runtime writes it with `LoadImage`, reads it back with
//! `StoreImage` and copies inside it with `MoveImage` (host callbacks
//! 0x64 to 0x66, `SFX.cs:1407-1414`); primitives sample it through a
//! texture page and a CLUT.

pub const VRAM_WIDTH: usize = 1024;
pub const VRAM_HEIGHT: usize = 512;

/// A rectangle in VRAM pixels (`RECT`: x, y, w, h).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i16,
    pub y: i16,
    pub w: i16,
    pub h: i16,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Vram {
    pixels: Vec<u16>,
    /// A `DR_AREA` render-to-texture pair has ended in this effect
    /// (`PSXTextureMgr.isCreateGenTexture`): from then on its 15-bit pages
    /// read the generated texture, not VRAM (`render::render_list`).
    pub generated_created: bool,
    /// `PSXTextureMgr.bgKey` shifted down 16 (texture page x in bits 0-3, y in
    /// bit 4) after the effect asked for a background capture (host command
    /// 120, `CaptureBG`); 15-bit pages with that place read the capture.
    pub background: Option<u16>,
    /// Frames [`render::render_list`] has drawn in this effect (`SFX.frameIndex`).
    pub render_frame: u32,
    /// The frame Detect's eye mesh first appeared in (`SFXMesh.DetectEyeFrameStart`).
    pub detect_eye_start: Option<u32>,
}

impl Default for Vram {
    fn default() -> Self {
        Self {
            pixels: vec![0; VRAM_WIDTH * VRAM_HEIGHT],
            generated_created: false,
            background: None,
            render_frame: 0,
            detect_eye_start: None,
        }
    }
}

impl std::fmt::Debug for Vram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vram").finish_non_exhaustive()
    }
}

/// VRAM index of (x, y), wrapped like the hardware (and like
/// `PSXTextureMgr`, which indexes `(y + i) * 1024 + x` inside the array).
fn index(x: i32, y: i32) -> usize {
    (y.rem_euclid(VRAM_HEIGHT as i32) as usize) * VRAM_WIDTH
        + x.rem_euclid(VRAM_WIDTH as i32) as usize
}

impl Vram {
    pub fn pixel(&self, x: i32, y: i32) -> u16 {
        self.pixels[index(x, y)]
    }

    pub fn pixels(&self) -> &[u16] {
        &self.pixels
    }

    /// `PSXTextureMgr.LoadImage`: `rect.w * rect.h` pixels, row by row.
    pub fn load_image(&mut self, rect: Rect, pixels: &[u16]) {
        let (w, h) = (i32::from(rect.w).max(0), i32::from(rect.h).max(0));
        for row in 0..h {
            for column in 0..w {
                if let Some(&value) = pixels.get((row * w + column) as usize) {
                    self.pixels[index(i32::from(rect.x) + column, i32::from(rect.y) + row)] = value;
                }
            }
        }
    }

    /// `PSXTextureMgr.StoreImage`: the pixels of `rect`, row by row.
    pub fn store_image(&self, rect: Rect) -> Vec<u16> {
        let (w, h) = (i32::from(rect.w).max(0), i32::from(rect.h).max(0));
        let mut out = Vec::with_capacity((w * h) as usize);
        for row in 0..h {
            for column in 0..w {
                out.push(self.pixel(i32::from(rect.x) + column, i32::from(rect.y) + row));
            }
        }
        out
    }

    /// `PSXTextureMgr.MoveImage`: copies `source` to (`x`, `y`).
    pub fn move_image(&mut self, source: Rect, x: i16, y: i16) {
        let pixels = self.store_image(source);
        self.load_image(
            Rect {
                x,
                y,
                w: source.w,
                h: source.h,
            },
            &pixels,
        );
    }

    /// A texture page as RGBA8, `width` texels wide and 256 tall: `depth`
    /// 0 is 4-bit and 1 8-bit (indices into the CLUT at `clut`), 2 is
    /// 15-bit direct. `tpage_x`/`tpage_y` are in VRAM pixels. Pixel 0 is
    /// transparent, as the PS1 draws it (`PSXTexture.GenTexture`).
    pub fn page_rgba(
        &self,
        tpage_x: i32,
        tpage_y: i32,
        depth: u8,
        clut: (i32, i32),
    ) -> (usize, Vec<u8>) {
        let width = match depth {
            0 => 256,
            1 => 128,
            _ => 64,
        };
        let mut out = Vec::with_capacity(width * 256 * 4);
        for y in 0..256 {
            for x in 0..width as i32 {
                let value = match depth {
                    0 => {
                        let word = self.pixel(tpage_x + x / 4, tpage_y + y);
                        let index = (word >> ((x % 4) * 4)) & 0xF;
                        self.pixel(clut.0 + i32::from(index), clut.1)
                    }
                    1 => {
                        let word = self.pixel(tpage_x + x / 2, tpage_y + y);
                        let index = (word >> ((x % 2) * 8)) & 0xFF;
                        self.pixel(clut.0 + i32::from(index), clut.1)
                    }
                    _ => self.pixel(tpage_x + x, tpage_y + y),
                };
                out.extend_from_slice(&rgba(value));
            }
        }
        (width, out)
    }

    /// The whole VRAM as 15-bit color, for looking at it.
    pub fn to_rgba(&self) -> Vec<u8> {
        self.pixels.iter().flat_map(|&value| rgba(value)).collect()
    }
}

/// A PS1 15-bit pixel as RGBA8 (`PSXTextureMgr._ConvertABGR16toABGR32`
/// shape: 5 bits each, alpha from the STP bit or nonzero color).
pub fn rgba(value: u16) -> [u8; 4] {
    let channel = |shift: u16| {
        let five = ((value >> shift) & 0x1F) as u8;
        (five << 3) | (five >> 2)
    };
    [
        channel(0),
        channel(5),
        channel(10),
        if value == 0 { 0 } else { 255 },
    ]
}
