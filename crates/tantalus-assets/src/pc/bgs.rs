use crate::install::ImportError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BgsCamera {
    pub projection: u16,
    pub rotation: [[i16; 3]; 3],
    pub translation: [i32; 3],
    pub center_offset: [i16; 2],
    pub width: i16,
    pub height: i16,
    pub viewport_x: [i16; 2],
    pub viewport_y: [i16; 2],
    pub depth_offset: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BgsSprite {
    pub clut_y: u16,
    pub clut_x: u16,
    pub texture_y: u8,
    pub texture_x: u8,
    pub resolution: u8,
    pub alpha: u8,
    pub v: u8,
    pub u: u8,
    pub height: u16,
    pub width: u16,
    pub transparency: u8,
    pub depth: u16,
    pub offset_y: u16,
    pub offset_x: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BgsOverlay {
    pub flags: u8,
    pub current_z: u16,
    pub original_z: u16,
    pub width: u16,
    pub height: u16,
    pub origin: [i16; 2],
    pub current: [i16; 2],
    pub limits: [i16; 4],
    pub screen: [i16; 2],
    pub scroll: [i16; 2],
    pub fraction: [i16; 2],
    pub camera: u8,
    pub is_x_offset: bool,
    pub viewport: u8,
    pub sprites: Vec<BgsSprite>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BgsLight {
    pub position: [i16; 3],
    pub padding: u16,
    pub color: [u8; 3],
    pub camera: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BgsAnimation {
    pub flags: u8,
    pub camera: u8,
    pub current_frame: u32,
    pub frame_rate: i16,
    pub counter: u16,
    pub frames: Vec<(u8, i8)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BgsFile {
    pub scene_length: u16,
    pub depth_bit_shift: u16,
    pub origin_z: i16,
    pub current_z: i16,
    pub origin: [i16; 2],
    pub current: [i16; 2],
    pub limits: [i16; 4],
    pub screen: [i16; 2],
    pub animations: Vec<BgsAnimation>,
    pub overlays: Vec<BgsOverlay>,
    pub lights: Vec<BgsLight>,
    pub cameras: Vec<BgsCamera>,
}

impl BgsFile {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut cursor = Cursor::at(bytes, 0, source)?;
        let scene_length = cursor.u16(source)?;
        if scene_length as usize != bytes.len() {
            return Err(invalid(
                source,
                format!(
                    "BGS scene length {scene_length} does not match {} bytes",
                    bytes.len()
                ),
            ));
        }
        let depth_bit_shift = cursor.u16(source)?;
        let animation_count = cursor.u16(source)? as usize;
        let overlay_count = cursor.u16(source)? as usize;
        let light_count = cursor.u16(source)? as usize;
        let camera_count = cursor.u16(source)? as usize;
        let animation_offset = cursor.u32(source)? as usize;
        let overlay_offset = cursor.u32(source)? as usize;
        let light_offset = cursor.u32(source)? as usize;
        let camera_offset = cursor.u32(source)? as usize;
        let origin_z = cursor.i16(source)?;
        let current_z = cursor.i16(source)?;
        let origin = cursor.pair(source)?;
        let current = cursor.pair(source)?;
        let limits = [
            cursor.i16(source)?,
            cursor.i16(source)?,
            cursor.i16(source)?,
            cursor.i16(source)?,
        ];
        let screen = cursor.pair(source)?;

        let mut overlays = Vec::with_capacity(overlay_count);
        let mut overlay_cursor = Cursor::at(bytes, overlay_offset, source)?;
        for _ in 0..overlay_count {
            let packed = overlay_cursor.u32(source)?;
            let flags = packed as u8;
            let current_z = (packed >> 8) as u16 & 0x0fff;
            let original_z = (packed >> 20) as u16 & 0x0fff;
            let width = overlay_cursor.u16(source)?;
            let height = overlay_cursor.u16(source)?;
            let origin = overlay_cursor.pair(source)?;
            let current = overlay_cursor.pair(source)?;
            let limits = [
                overlay_cursor.i16(source)?,
                overlay_cursor.i16(source)?,
                overlay_cursor.i16(source)?,
                overlay_cursor.i16(source)?,
            ];
            let screen = overlay_cursor.pair(source)?;
            let scroll = overlay_cursor.pair(source)?;
            let fraction = overlay_cursor.pair(source)?;
            let packed = overlay_cursor.u32(source)?;
            let mut bit = 0;
            let camera = read_bits(packed, &mut bit, 8) as u8;
            let is_x_offset = read_bits(packed, &mut bit, 1) != 0;
            let viewport = read_bits(packed, &mut bit, 7) as u8;
            let sprite_count = read_bits(packed, &mut bit, 16) as usize;
            let location_offset = overlay_cursor.u32(source)? as usize;
            let parameter_offset = overlay_cursor.u32(source)? as usize;
            overlay_cursor.u32(source)?;
            overlay_cursor.u32(source)?;
            if camera_count == 0 || camera as usize >= camera_count {
                return Err(invalid(
                    source,
                    format!("overlay references invalid camera {camera}"),
                ));
            }
            let mut parameter_cursor = Cursor::at(bytes, parameter_offset, source)?;
            let mut location_cursor = Cursor::at(bytes, location_offset, source)?;
            let mut sprites = Vec::with_capacity(sprite_count);
            for _ in 0..sprite_count {
                let first = parameter_cursor.u32(source)?;
                bit = 0;
                let clut_y = read_bits(first, &mut bit, 9) as u16;
                let clut_x = read_bits(first, &mut bit, 6) as u16;
                let texture_y = read_bits(first, &mut bit, 1) as u8;
                let texture_x = read_bits(first, &mut bit, 4) as u8;
                let resolution = read_bits(first, &mut bit, 2) as u8;
                let alpha = read_bits(first, &mut bit, 2) as u8;
                let v = read_bits(first, &mut bit, 8) as u8;
                let second = parameter_cursor.u32(source)?;
                bit = 0;
                let u = read_bits(second, &mut bit, 8) as u8;
                let height = read_bits(second, &mut bit, 10) as u16;
                let width = read_bits(second, &mut bit, 10) as u16;
                let transparency = read_bits(second, &mut bit, 1) as u8;
                read_bits(second, &mut bit, 3);
                let packed = location_cursor.u32(source)?;
                bit = 0;
                let depth = read_bits(packed, &mut bit, 12) as u16;
                let offset_y = read_bits(packed, &mut bit, 10) as u16;
                let offset_x = read_bits(packed, &mut bit, 10) as u16;
                sprites.push(BgsSprite {
                    clut_y,
                    clut_x,
                    texture_y,
                    texture_x,
                    resolution,
                    alpha,
                    v,
                    u,
                    height,
                    width,
                    transparency,
                    depth,
                    offset_y,
                    offset_x,
                });
            }
            overlays.push(BgsOverlay {
                flags,
                current_z,
                original_z,
                width,
                height,
                origin,
                current,
                limits,
                screen,
                scroll,
                fraction,
                camera,
                is_x_offset,
                viewport,
                sprites,
            });
        }

        let mut lights = Vec::with_capacity(light_count);
        let mut light_cursor = Cursor::at(bytes, light_offset, source)?;
        for _ in 0..light_count {
            let position = light_cursor.vec3(source)?;
            let padding = light_cursor.u16(source)?;
            let color = light_cursor.byte_array(source)?;
            let camera = light_cursor.byte(source)?;
            lights.push(BgsLight {
                position,
                padding,
                color,
                camera,
            });
        }

        let mut cameras = Vec::with_capacity(camera_count);
        let mut camera_cursor = Cursor::at(bytes, camera_offset, source)?;
        for _ in 0..camera_count {
            let projection = camera_cursor.u16(source)?;
            let rotation = [
                camera_cursor.vec3(source)?,
                camera_cursor.vec3(source)?,
                camera_cursor.vec3(source)?,
            ];
            let translation = [
                camera_cursor.i32(source)?,
                camera_cursor.i32(source)?,
                camera_cursor.i32(source)?,
            ];
            let center_offset = camera_cursor.pair(source)?;
            let width = camera_cursor.i16(source)?;
            let height = camera_cursor.i16(source)?;
            let viewport_x = camera_cursor.pair(source)?;
            let viewport_y = camera_cursor.pair(source)?;
            let depth_offset = camera_cursor.i32(source)?;
            cameras.push(BgsCamera {
                projection,
                rotation,
                translation,
                center_offset,
                width,
                height,
                viewport_x,
                viewport_y,
                depth_offset,
            });
        }

        let mut animations = Vec::with_capacity(animation_count);
        let mut animation_cursor = Cursor::at(bytes, animation_offset, source)?;
        for _ in 0..animation_count {
            let packed = animation_cursor.u32(source)?;
            let flags = packed as u8;
            let frame_count = (packed >> 8) as usize;
            if frame_count > 100_000 {
                return Err(invalid(source, "animation frame count exceeds its budget"));
            }
            let packed = animation_cursor.u32(source)?;
            let camera = packed as u8;
            let current_frame = packed >> 8;
            let frame_rate = animation_cursor.i16(source)?;
            let counter = animation_cursor.u16(source)?;
            let frame_offset = animation_cursor.u32(source)? as usize;
            let mut frame_cursor = Cursor::at(bytes, frame_offset, source)?;
            let mut frames = Vec::with_capacity(frame_count);
            for _ in 0..frame_count {
                frames.push((frame_cursor.byte(source)?, frame_cursor.byte(source)? as i8));
            }
            animations.push(BgsAnimation {
                flags,
                camera,
                current_frame,
                frame_rate,
                counter,
                frames,
            });
        }

        Ok(Self {
            scene_length,
            depth_bit_shift,
            origin_z,
            current_z,
            origin,
            current,
            limits,
            screen,
            animations,
            overlays,
            lights,
            cameras,
        })
    }
}

fn read_bits(input: u32, bit: &mut u8, count: u8) -> u32 {
    let mask = (1_u32 << count) - 1;
    let value = (input >> *bit) & mask;
    *bit += count;
    value
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn at(bytes: &'a [u8], position: usize, source: &str) -> Result<Self, ImportError> {
        if position > bytes.len() {
            return Err(invalid(source, "BGS offset lies outside the scene"));
        }
        Ok(Self { bytes, position })
    }

    fn array<const N: usize>(&mut self, source: &str) -> Result<[u8; N], ImportError> {
        let end = self
            .position
            .checked_add(N)
            .ok_or_else(|| invalid(source, "BGS cursor overflow"))?;
        let data = self.bytes.get(self.position..end).ok_or_else(|| {
            invalid(
                source,
                format!("BGS is truncated at byte {}", self.position),
            )
        })?;
        self.position = end;
        Ok(data.try_into().expect("fixed BGS field"))
    }

    fn byte(&mut self, source: &str) -> Result<u8, ImportError> {
        Ok(self.array::<1>(source)?[0])
    }

    fn u16(&mut self, source: &str) -> Result<u16, ImportError> {
        Ok(u16::from_le_bytes(self.array(source)?))
    }

    fn i16(&mut self, source: &str) -> Result<i16, ImportError> {
        Ok(i16::from_le_bytes(self.array(source)?))
    }

    fn u32(&mut self, source: &str) -> Result<u32, ImportError> {
        Ok(u32::from_le_bytes(self.array(source)?))
    }

    fn i32(&mut self, source: &str) -> Result<i32, ImportError> {
        Ok(i32::from_le_bytes(self.array(source)?))
    }

    fn pair(&mut self, source: &str) -> Result<[i16; 2], ImportError> {
        Ok([self.i16(source)?, self.i16(source)?])
    }

    fn vec3(&mut self, source: &str) -> Result<[i16; 3], ImportError> {
        Ok([self.i16(source)?, self.i16(source)?, self.i16(source)?])
    }

    fn byte_array(&mut self, source: &str) -> Result<[u8; 3], ImportError> {
        Ok([self.byte(source)?, self.byte(source)?, self.byte(source)?])
    }
}

fn invalid(source: &str, message: impl Into<String>) -> ImportError {
    ImportError::invalid(source, message.into())
}
