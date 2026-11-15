use crate::install::ImportError;
use serde::{Deserialize, Serialize};

pub const DXT5_FORMAT: i32 = 12;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextureSettings {
    pub filter_mode: i32,
    pub anisotropy: i32,
    pub mip_bias: f32,
    pub wrap_mode: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Texture2D {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub complete_image_size: i32,
    pub format: i32,
    pub mip_count: i32,
    pub is_readable: bool,
    pub read_allowed: bool,
    pub image_count: i32,
    pub texture_dimension: i32,
    pub settings: TextureSettings,
    pub lightmap_format: i32,
    pub color_space: i32,
    pub image_data: Vec<u8>,
}

impl Texture2D {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut cursor = Cursor { bytes, position: 0 };
        let name = cursor.string(source)?;
        let width = cursor.i32(source)?;
        let height = cursor.i32(source)?;
        let complete_image_size = cursor.i32(source)?;
        let format = cursor.i32(source)?;
        let mip_count = cursor.i32(source)?;
        let is_readable = cursor.byte(source)? != 0;
        let read_allowed = cursor.byte(source)? != 0;
        cursor.align(source)?;
        let image_count = cursor.i32(source)?;
        let texture_dimension = cursor.i32(source)?;
        let settings = TextureSettings {
            filter_mode: cursor.i32(source)?,
            anisotropy: cursor.i32(source)?,
            mip_bias: cursor.f32(source)?,
            wrap_mode: cursor.i32(source)?,
        };
        let lightmap_format = cursor.i32(source)?;
        let color_space = cursor.i32(source)?;
        let image_data = cursor.byte_array(source)?;
        if cursor.position != bytes.len() {
            return Err(invalid(source, "Texture2D has unexpected trailing data"));
        }
        if width <= 0 || height <= 0 || width > 16_384 || height > 16_384 {
            return Err(invalid(
                source,
                "Texture2D dimensions are outside supported bounds",
            ));
        }
        if complete_image_size < 0 || complete_image_size as usize != image_data.len() {
            return Err(invalid(
                source,
                "Texture2D complete image size does not match its data",
            ));
        }
        if format != DXT5_FORMAT {
            return Err(ImportError::unsupported(
                source,
                format!("Texture2D format {format}"),
            ));
        }
        let expected = (width as usize).div_ceil(4) * (height as usize).div_ceil(4) * 16;
        if image_data.len() != expected {
            return Err(invalid(
                source,
                format!(
                    "DXT5 data has {} bytes for {width}x{height}, expected {expected}",
                    image_data.len()
                ),
            ));
        }
        if mip_count <= 0 || image_count != 1 || texture_dimension != 2 {
            return Err(invalid(
                source,
                "Texture2D mip/image/dimension declaration is unsupported",
            ));
        }
        Ok(Self {
            name,
            width,
            height,
            complete_image_size,
            format,
            mip_count,
            is_readable,
            read_allowed,
            image_count,
            texture_dimension,
            settings,
            lightmap_format,
            color_space,
            image_data,
        })
    }

    pub fn rgba_bytes(&self) -> Result<Vec<u8>, ImportError> {
        let decoded = decode_dxt5(self)?;
        let width = self.width as usize;
        let height = self.height as usize;
        let stride = width * 4;
        let mut oriented = vec![0; decoded.len()];
        for row in 0..height {
            let source = row * stride;
            let target = (height - row - 1) * stride;
            oriented[target..target + stride].copy_from_slice(&decoded[source..source + stride]);
        }
        Ok(oriented)
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn string(&mut self, source: &str) -> Result<String, ImportError> {
        let bytes = self.byte_array(source)?;
        String::from_utf8(bytes)
            .map_err(|error| invalid(source, format!("Texture2D name is not UTF-8: {error}")))
    }

    fn byte_array(&mut self, source: &str) -> Result<Vec<u8>, ImportError> {
        let length = self.i32(source)?;
        let length = usize::try_from(length)
            .map_err(|_| invalid(source, "Texture2D array has negative length"))?;
        if length > 256 * 1024 * 1024 {
            return Err(invalid(source, "Texture2D array exceeds the 256 MiB limit"));
        }
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| invalid(source, "Texture2D array range overflow"))?;
        let data = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| invalid(source, "Texture2D array is truncated"))?
            .to_vec();
        self.position = end;
        self.align(source)?;
        Ok(data)
    }

    fn i32(&mut self, source: &str) -> Result<i32, ImportError> {
        Ok(i32::from_le_bytes(self.array(source)?))
    }

    fn f32(&mut self, source: &str) -> Result<f32, ImportError> {
        Ok(f32::from_le_bytes(self.array(source)?))
    }

    fn byte(&mut self, source: &str) -> Result<u8, ImportError> {
        Ok(self.array::<1>(source)?[0])
    }

    fn array<const N: usize>(&mut self, source: &str) -> Result<[u8; N], ImportError> {
        let end = self
            .position
            .checked_add(N)
            .ok_or_else(|| invalid(source, "Texture2D cursor overflow"))?;
        let data = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| invalid(source, "Texture2D is truncated"))?;
        self.position = end;
        Ok(data.try_into().expect("fixed Texture2D field"))
    }

    fn align(&mut self, source: &str) -> Result<(), ImportError> {
        let padding = (4 - self.position % 4) % 4;
        let end = self.position + padding;
        if end > self.bytes.len() {
            return Err(invalid(source, "Texture2D alignment is truncated"));
        }
        self.position = end;
        Ok(())
    }
}

fn decode_dxt5(texture: &Texture2D) -> Result<Vec<u8>, ImportError> {
    let width = texture.width as usize;
    let height = texture.height as usize;
    let mut output = vec![0; width * height * 4];
    let blocks_per_row = width.div_ceil(4);
    for (block_index, block) in texture.image_data.chunks_exact(16).enumerate() {
        let block_x = (block_index % blocks_per_row) * 4;
        let block_y = (block_index / blocks_per_row) * 4;
        let alpha = alpha_palette(block[0], block[1]);
        let alpha_indices = u64::from_le_bytes([
            block[2], block[3], block[4], block[5], block[6], block[7], 0, 0,
        ]);
        let colors = color_palette(
            u16::from_le_bytes([block[8], block[9]]),
            u16::from_le_bytes([block[10], block[11]]),
        );
        let color_indices = u32::from_le_bytes([block[12], block[13], block[14], block[15]]);
        for y in 0..4 {
            for x in 0..4 {
                let pixel_x = block_x + x;
                let pixel_y = block_y + y;
                if pixel_x >= width || pixel_y >= height {
                    continue;
                }
                let local = y * 4 + x;
                let color = colors[((color_indices >> (local * 2)) & 3) as usize];
                let alpha = alpha[((alpha_indices >> (local * 3)) & 7) as usize];
                let target = (pixel_y * width + pixel_x) * 4;
                output[target..target + 4].copy_from_slice(&[color[0], color[1], color[2], alpha]);
            }
        }
    }
    Ok(output)
}

fn alpha_palette(first: u8, second: u8) -> [u8; 8] {
    let mut values = [0; 8];
    values[0] = first;
    values[1] = second;
    if first > second {
        for index in 1..=6 {
            values[index + 1] =
                (((7 - index) as u16 * first as u16 + index as u16 * second as u16) / 7) as u8;
        }
    } else {
        for index in 1..=4 {
            values[index + 1] =
                (((5 - index) as u16 * first as u16 + index as u16 * second as u16) / 5) as u8;
        }
        values[6] = 0;
        values[7] = 255;
    }
    values
}

fn color_palette(first: u16, second: u16) -> [[u8; 3]; 4] {
    let first = rgb565(first);
    let second = rgb565(second);
    let mut colors = [first, second, [0; 3], [0; 3]];
    for channel in 0..3 {
        colors[2][channel] =
            ((2 * u16::from(first[channel]) + u16::from(second[channel])) / 3) as u8;
        colors[3][channel] =
            ((u16::from(first[channel]) + 2 * u16::from(second[channel])) / 3) as u8;
    }
    colors
}

fn rgb565(value: u16) -> [u8; 3] {
    [
        (((value >> 11) & 31) as u8) << 3 | (((value >> 11) & 31) as u8) >> 2,
        (((value >> 5) & 63) as u8) << 2 | (((value >> 5) & 63) as u8) >> 4,
        ((value & 31) as u8) << 3 | ((value & 31) as u8) >> 2,
    ]
}

fn invalid(source: &str, message: impl Into<String>) -> ImportError {
    ImportError::invalid(source, message.into())
}
