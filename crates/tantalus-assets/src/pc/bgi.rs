use crate::install::ImportError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BgiTriangle {
    pub flags: u16,
    pub data: u16,
    pub floor: i16,
    pub normal: i16,
    pub theta_x: i16,
    pub theta_z: i16,
    pub vertices: [i16; 3],
    pub edges: [i16; 3],
    pub neighbors: [i16; 3],
    pub center: [i16; 3],
    pub d: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BgiFloor {
    pub flags: u16,
    pub index: u16,
    pub bounds: [[i16; 3]; 4],
    pub triangles: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BgiAnimation {
    pub flags: u16,
    pub frame_rate: i16,
    pub counter: u16,
    pub current_frame: i32,
    pub frame_values: Vec<i16>,
    pub triangles: Vec<Vec<i32>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BgiFile {
    pub origin: [i16; 3],
    pub current: [i16; 3],
    pub minimum: [i16; 3],
    pub maximum: [i16; 3],
    pub character: [i16; 3],
    pub active_floor: i16,
    pub active_triangle: i16,
    pub triangles: Vec<BgiTriangle>,
    pub edges: Vec<(u16, i16)>,
    pub animations: Vec<BgiAnimation>,
    pub floors: Vec<BgiFloor>,
    pub normals: Vec<[i32; 4]>,
    pub vertices: Vec<[i16; 3]>,
}

impl BgiFile {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut cursor = Cursor::at(bytes, 0, source)?;
        if cursor.u32(source)? != 0xacdc_dead {
            return Err(invalid(source, "BGI magic does not match 0xACDCDEAD"));
        }
        let data_size = cursor.u16(source)? as usize;
        if data_size + 4 != bytes.len() {
            return Err(invalid(
                source,
                format!(
                    "BGI data size {data_size} does not match {} payload bytes",
                    bytes.len() - 4
                ),
            ));
        }
        let origin = cursor.vec3(source)?;
        let current = cursor.vec3(source)?;
        let minimum = cursor.vec3(source)?;
        let maximum = cursor.vec3(source)?;
        let character = cursor.vec3(source)?;
        let active_floor = cursor.i16(source)?;
        let active_triangle = cursor.i16(source)?;
        let tri_count = cursor.u16(source)? as usize;
        let tri_offset = cursor.u16(source)?;
        let edge_count = cursor.u16(source)? as usize;
        let edge_offset = cursor.u16(source)?;
        let animation_count = cursor.u16(source)? as usize;
        let animation_offset = cursor.u16(source)?;
        let floor_count = cursor.u16(source)? as usize;
        let floor_offset = cursor.u16(source)?;
        let normal_count = cursor.u16(source)? as usize;
        let normal_offset = cursor.u16(source)?;
        let vertex_count = cursor.u16(source)? as usize;
        let vertex_offset = cursor.u16(source)?;

        let mut triangles = Vec::with_capacity(tri_count);
        let mut triangle_cursor = Cursor::at(bytes, 4 + tri_offset as usize, source)?;
        for _ in 0..tri_count {
            let triangle = BgiTriangle {
                flags: triangle_cursor.u16(source)?,
                data: triangle_cursor.u16(source)?,
                floor: triangle_cursor.i16(source)?,
                normal: triangle_cursor.i16(source)?,
                theta_x: triangle_cursor.i16(source)?,
                theta_z: triangle_cursor.i16(source)?,
                vertices: triangle_cursor.i16_array(source)?,
                edges: triangle_cursor.i16_array(source)?,
                neighbors: triangle_cursor.i16_array(source)?,
                center: triangle_cursor.vec3(source)?,
                d: triangle_cursor.i32(source)?,
            };
            for vertex in triangle.vertices {
                if vertex < 0 || vertex as usize >= vertex_count {
                    return Err(invalid(
                        source,
                        format!("triangle references invalid vertex {vertex}"),
                    ));
                }
            }
            triangles.push(triangle);
        }
        let mut edges = Vec::with_capacity(edge_count);
        let mut edge_cursor = Cursor::at(bytes, 4 + edge_offset as usize, source)?;
        for _ in 0..edge_count {
            edges.push((edge_cursor.u16(source)?, edge_cursor.i16(source)?));
        }

        let mut floors = Vec::with_capacity(floor_count);
        let mut floor_cursor = Cursor::at(bytes, 4 + floor_offset as usize, source)?;
        for _ in 0..floor_count {
            let flags = floor_cursor.u16(source)?;
            let index = floor_cursor.u16(source)?;
            let bounds = [
                floor_cursor.vec3(source)?,
                floor_cursor.vec3(source)?,
                floor_cursor.vec3(source)?,
                floor_cursor.vec3(source)?,
            ];
            let count = floor_cursor.u16(source)? as usize;
            let offset = floor_cursor.u16(source)?;
            let triangles = read_i32_list(bytes, offset, count, source)?;
            validate_triangles(&triangles, tri_count, source)?;
            floors.push(BgiFloor {
                flags,
                index,
                bounds,
                triangles,
            });
        }

        let mut normals = Vec::with_capacity(normal_count);
        let mut normal_cursor = Cursor::at(bytes, 4 + normal_offset as usize, source)?;
        for _ in 0..normal_count {
            normals.push(normal_cursor.i32_array(source)?);
        }
        let mut vertices = Vec::with_capacity(vertex_count);
        let mut vertex_cursor = Cursor::at(bytes, 4 + vertex_offset as usize, source)?;
        for _ in 0..vertex_count {
            vertices.push(vertex_cursor.vec3(source)?);
        }

        let mut animations = Vec::with_capacity(animation_count);
        let mut animation_cursor = Cursor::at(bytes, 4 + animation_offset as usize, source)?;
        for _ in 0..animation_count {
            let flags = animation_cursor.u16(source)?;
            let frame_count = animation_cursor.u16(source)? as usize;
            let frame_rate = animation_cursor.i16(source)?;
            let counter = animation_cursor.u16(source)?;
            let current_frame = animation_cursor.i32(source)?;
            let frame_offset = animation_cursor.u32(source)? as usize;
            let mut frame_cursor = Cursor::at(bytes, 4 + frame_offset, source)?;
            let mut frame_values = Vec::with_capacity(frame_count);
            let mut triangle_lists = Vec::with_capacity(frame_count);
            for _ in 0..frame_count {
                let _flags = frame_cursor.u16(source)?;
                frame_values.push(frame_cursor.i16(source)?);
                let count = frame_cursor.u16(source)? as usize;
                let offset = frame_cursor.u16(source)?;
                let triangles = read_i32_list(bytes, offset, count, source)?;
                validate_triangles(&triangles, tri_count, source)?;
                triangle_lists.push(triangles);
            }
            animations.push(BgiAnimation {
                flags,
                frame_rate,
                counter,
                current_frame,
                frame_values,
                triangles: triangle_lists,
            });
        }

        Ok(Self {
            origin,
            current,
            minimum,
            maximum,
            character,
            active_floor,
            active_triangle,
            triangles,
            edges,
            animations,
            floors,
            normals,
            vertices,
        })
    }
}

fn read_i32_list(
    bytes: &[u8],
    offset: u16,
    count: usize,
    source: &str,
) -> Result<Vec<i32>, ImportError> {
    let mut cursor = Cursor::at(bytes, 4 + offset as usize, source)?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(cursor.i32(source)?);
    }
    Ok(values)
}

fn validate_triangles(values: &[i32], count: usize, source: &str) -> Result<(), ImportError> {
    for index in values {
        if *index < 0 || *index as usize >= count {
            return Err(invalid(
                source,
                format!("invalid triangle reference {index}"),
            ));
        }
    }
    Ok(())
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn at(bytes: &'a [u8], position: usize, source: &str) -> Result<Self, ImportError> {
        if position > bytes.len() {
            return Err(invalid(source, "BGI offset lies outside the payload"));
        }
        Ok(Self { bytes, position })
    }

    fn array<const N: usize>(&mut self, source: &str) -> Result<[u8; N], ImportError> {
        let end = self
            .position
            .checked_add(N)
            .ok_or_else(|| invalid(source, "BGI cursor overflow"))?;
        let bytes = self.bytes.get(self.position..end).ok_or_else(|| {
            invalid(
                source,
                format!("BGI data is truncated at byte {}", self.position),
            )
        })?;
        self.position = end;
        Ok(bytes.try_into().expect("fixed BGI field"))
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

    fn vec3(&mut self, source: &str) -> Result<[i16; 3], ImportError> {
        self.i16_array(source)
    }

    fn i16_array(&mut self, source: &str) -> Result<[i16; 3], ImportError> {
        Ok([self.i16(source)?, self.i16(source)?, self.i16(source)?])
    }

    fn i32_array(&mut self, source: &str) -> Result<[i32; 4], ImportError> {
        Ok([
            self.i32(source)?,
            self.i32(source)?,
            self.i32(source)?,
            self.i32(source)?,
        ])
    }
}

fn invalid(source: &str, message: impl Into<String>) -> ImportError {
    ImportError::invalid(source, message.into())
}
