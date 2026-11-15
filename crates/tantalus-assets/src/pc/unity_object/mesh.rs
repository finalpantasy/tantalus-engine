//! Unity 5.2 `Mesh` (class 43) decoded from its type tree.
//!
//! Unity 5.2 stores no explicit stream table: streams are derived from the
//! channel list (per stream, stride = sum of channel sizes; each stream's data
//! is `vertex_count * stride` bytes, and the next stream starts at a 16-byte
//! boundary). Index buffers are 16-bit. Both rules were checked against every
//! mesh in `p0data4.bin`: the derived layout must account for the vertex blob.

use super::fields::{Aabb, Fields};
use super::type_tree::Value;
use crate::install::ImportError;
use serde::{Deserialize, Serialize};

pub const MESH_CLASS_ID: u16 = 43;
/// Unity 5.x vertex channel slots.
pub const CHANNEL_NAMES: [&str; 8] = [
    "position", "normal", "color", "uv0", "uv1", "uv2", "uv3", "tangent",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VertexChannel {
    pub stream: u8,
    pub offset: u8,
    pub format: u8,
    pub dimension: u8,
}

impl VertexChannel {
    /// Component size for the pre-2017 `VertexChannelFormat` enum:
    /// 0 float, 1 half, 2 color (unorm8), 3 byte, 4 uint32.
    pub fn component_size(&self) -> Option<usize> {
        match self.format {
            0 | 4 => Some(4),
            1 => Some(2),
            2 | 3 => Some(1),
            _ => None,
        }
    }

    pub fn byte_size(&self) -> Option<usize> {
        self.component_size()
            .map(|size| size * usize::from(self.dimension))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VertexStream {
    pub channel_mask: u32,
    pub offset: usize,
    pub stride: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SubMesh {
    pub first_byte: u32,
    pub index_count: u32,
    pub topology: i32,
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub local_aabb: Aabb,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoneInfluence {
    pub weights: [f32; 4],
    pub bones: [i32; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub name: String,
    pub vertex_count: u32,
    pub current_channels: u32,
    pub channels: Vec<VertexChannel>,
    pub streams: Vec<VertexStream>,
    pub vertex_data: Vec<u8>,
    pub indices: Vec<u16>,
    pub submeshes: Vec<SubMesh>,
    pub bind_poses: Vec<[f32; 16]>,
    pub bone_name_hashes: Vec<u32>,
    pub root_bone_name_hash: u32,
    pub skin: Vec<BoneInfluence>,
    pub blend_shape_vertices: usize,
    pub blend_shape_channels: usize,
    pub mesh_compression: u8,
    pub compressed_vertex_items: u32,
    pub local_aabb: Aabb,
}

impl Mesh {
    pub fn from_value(value: &Value<'_>, source: &str) -> Result<Self, ImportError> {
        let fields = Fields::new(value, source, "Mesh");
        let name = fields.string("m_Name")?;
        let vertex_data = fields.sub("m_VertexData")?;
        let vertex_count = vertex_data.u32("m_VertexCount")?;
        let current_channels = vertex_data.u32("m_CurrentChannels")?;
        let mut channels = Vec::new();
        for channel in vertex_data.array("m_Channels")? {
            let channel = Fields::new(channel, source, "Mesh channel");
            channels.push(VertexChannel {
                stream: channel.u32("stream")? as u8,
                offset: channel.u32("offset")? as u8,
                format: channel.u32("format")? as u8,
                dimension: channel.u32("dimension")? as u8,
            });
        }
        let blob = vertex_data.bytes("m_DataSize")?.to_vec();
        let streams = derive_streams(&channels, vertex_count, blob.len())
            .map_err(|message| fields.error(message))?;

        let index_bytes = fields.bytes("m_IndexBuffer")?;
        if index_bytes.len() % 2 != 0 {
            return Err(fields.error("index buffer has an odd byte length"));
        }
        let indices: Vec<u16> = index_bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();

        let mut submeshes = Vec::new();
        for submesh in fields.array("m_SubMeshes")? {
            let sub = Fields::new(submesh, source, "SubMesh");
            submeshes.push(SubMesh {
                first_byte: sub.u32("firstByte")?,
                index_count: sub.u32("indexCount")?,
                topology: sub.i32("topology")?,
                first_vertex: sub.u32("firstVertex")?,
                vertex_count: sub.u32("vertexCount")?,
                local_aabb: sub.aabb("localAABB")?,
            });
        }
        for (number, submesh) in submeshes.iter().enumerate() {
            if submesh.first_byte % 2 != 0 {
                return Err(fields.error(format!("submesh {number} starts on an odd byte")));
            }
            let start = submesh.first_byte as usize / 2;
            let end = start + submesh.index_count as usize;
            let range = indices.get(start..end).ok_or_else(|| {
                fields.error(format!(
                    "submesh {number} indices {start}..{end} exceed {} indices",
                    indices.len()
                ))
            })?;
            let vertex_end = u64::from(submesh.first_vertex) + u64::from(submesh.vertex_count);
            if vertex_end > u64::from(vertex_count) {
                return Err(fields.error(format!(
                    "submesh {number} vertex range ends at {vertex_end} of {vertex_count}"
                )));
            }
            if let Some(bad) = range
                .iter()
                .find(|index| u32::from(**index) >= vertex_count)
            {
                return Err(fields.error(format!(
                    "submesh {number} index {bad} is out of range for {vertex_count} vertices"
                )));
            }
        }

        let mut bind_poses = Vec::new();
        for matrix in fields.array("m_BindPose")? {
            let matrix = Fields::new(matrix, source, "bind pose");
            let mut out = [0.0_f32; 16];
            for (index, slot) in out.iter_mut().enumerate() {
                *slot = matrix.f32(&format!("e{}{}", index / 4, index % 4))?;
            }
            bind_poses.push(out);
        }
        let bone_name_hashes = fields.u32s("m_BoneNameHashes")?;
        let mut skin = Vec::new();
        for influence in fields.array("m_Skin")? {
            let influence = Fields::new(influence, source, "bone influence");
            let mut weights = [0.0; 4];
            let mut bones = [0; 4];
            for slot in 0..4 {
                weights[slot] = influence.f32(&format!("weight[{slot}]"))?;
                bones[slot] = influence.i32(&format!("boneIndex[{slot}]"))?;
            }
            skin.push(BoneInfluence { weights, bones });
        }
        if !skin.is_empty() && skin.len() != vertex_count as usize {
            return Err(fields.error(format!(
                "{} bone influences for {vertex_count} vertices",
                skin.len()
            )));
        }
        for (vertex, influence) in skin.iter().enumerate() {
            for slot in 0..4 {
                let bone = influence.bones[slot];
                if influence.weights[slot] != 0.0 && (bone < 0 || bone as usize >= bind_poses.len())
                {
                    return Err(fields.error(format!(
                        "vertex {vertex} weights bone {bone} of {} bind poses",
                        bind_poses.len()
                    )));
                }
            }
        }
        let shapes = fields.sub("m_Shapes")?;
        let compressed = fields.sub("m_CompressedMesh")?;
        Ok(Self {
            name,
            vertex_count,
            current_channels,
            channels,
            streams,
            vertex_data: blob,
            indices,
            submeshes,
            bind_poses,
            bone_name_hashes,
            root_bone_name_hash: fields.u32("m_RootBoneNameHash")?,
            skin,
            blend_shape_vertices: shapes.len("vertices")?,
            blend_shape_channels: shapes.len("channels")?,
            mesh_compression: fields.u32("m_MeshCompression")? as u8,
            compressed_vertex_items: compressed.sub("m_Vertices")?.u32("m_NumItems")?,
            local_aabb: fields.aabb("m_LocalAABB")?,
        })
    }

    /// Channel layout as text, e.g. `position:f32x3@0/0`.
    pub fn layout_signature(&self) -> String {
        let mut parts = Vec::new();
        for (slot, channel) in self.channels.iter().enumerate() {
            if channel.dimension == 0 {
                continue;
            }
            let format = match channel.format {
                0 => "f32",
                1 => "f16",
                2 => "unorm8",
                3 => "u8",
                4 => "u32",
                _ => "unknown",
            };
            parts.push(format!(
                "{}:{format}x{}@{}/{}",
                CHANNEL_NAMES.get(slot).copied().unwrap_or("extra"),
                channel.dimension,
                channel.stream,
                channel.offset
            ));
        }
        parts.join(",")
    }

    /// Decodes one channel as `f32` components (`dimension` per vertex).
    pub fn channel_f32(&self, slot: usize) -> Option<Vec<f32>> {
        let channel = self.channels.get(slot)?;
        if channel.dimension == 0 {
            return None;
        }
        let stream = self.streams.get(usize::from(channel.stream))?;
        let size = channel.component_size()?;
        let mut out =
            Vec::with_capacity(self.vertex_count as usize * usize::from(channel.dimension));
        for vertex in 0..self.vertex_count as usize {
            let base = stream.offset + vertex * stream.stride + usize::from(channel.offset);
            for component in 0..usize::from(channel.dimension) {
                let at = base + component * size;
                let bytes = self.vertex_data.get(at..at + size)?;
                out.push(match channel.format {
                    0 => f32::from_le_bytes(bytes.try_into().ok()?),
                    1 => half_to_f32(u16::from_le_bytes([bytes[0], bytes[1]])),
                    2 => f32::from(bytes[0]) / 255.0,
                    3 => f32::from(bytes[0]),
                    4 => u32::from_le_bytes(bytes.try_into().ok()?) as f32,
                    _ => return None,
                });
            }
        }
        Some(out)
    }
}

/// Derives the Unity 5.x stream table and requires it to fit the vertex blob.
fn derive_streams(
    channels: &[VertexChannel],
    vertex_count: u32,
    blob_len: usize,
) -> Result<Vec<VertexStream>, String> {
    let stream_count = channels
        .iter()
        .filter(|channel| channel.dimension > 0)
        .map(|channel| usize::from(channel.stream) + 1)
        .max()
        .unwrap_or(0);
    let mut streams = Vec::with_capacity(stream_count);
    let mut offset = 0_usize;
    let mut unaligned_end = 0_usize;
    for stream in 0..stream_count {
        let mut mask = 0_u32;
        let mut stride = 0_usize;
        for (slot, channel) in channels.iter().enumerate() {
            if usize::from(channel.stream) != stream || channel.dimension == 0 {
                continue;
            }
            let size = channel
                .byte_size()
                .ok_or_else(|| format!("channel {slot} has unknown format {}", channel.format))?;
            mask |= 1 << slot;
            stride += size;
        }
        for (slot, channel) in channels.iter().enumerate() {
            if usize::from(channel.stream) == stream && channel.dimension > 0 {
                let end = usize::from(channel.offset) + channel.byte_size().unwrap_or(0);
                if end > stride {
                    return Err(format!(
                        "channel {slot} ends at byte {end} beyond stream stride {stride}"
                    ));
                }
            }
        }
        streams.push(VertexStream {
            channel_mask: mask,
            offset,
            stride,
        });
        offset += vertex_count as usize * stride;
        unaligned_end = offset;
        offset = (offset + 15) & !15;
    }
    if blob_len != unaligned_end && blob_len != offset {
        return Err(format!(
            "vertex blob has {blob_len} bytes; derived stream layout needs {unaligned_end} (aligned {offset})"
        ));
    }
    Ok(streams)
}

fn half_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        31 if mantissa == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        _ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}
