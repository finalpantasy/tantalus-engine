//! Terrain collision: the downward ray against a block's walk meshes.
//!
//! The source has no separate collision data. Every block mesh (terrain, object,
//! beach, river, sea, ...) is a walk mesh, and the per-triangle map id is the
//! `tangent.x` of the triangle's first vertex (`WMBlock.cs:23` `AddWalkMesh` keeps the
//! tangents; `WMBlock.cs:165`, `WMPhysics.cs:15` read them). The map id packs
//! event (bits 14-15), area (bits 8-13) and topograph (bits 2-7); the topograph
//! decides walkability (see `movement`).

use super::coords::{BLOCK_COLUMNS, BLOCK_ROWS, BlockLayout};
use super::vec3::Vec3;

/// Map ids `WMPhysics.Raycast` skips unless `IgnoreExceptions`
/// (`WM/WMPhysics.cs:15-17`: 4078, 4088, 2040).
pub const EXCLUDED_MAP_IDS: [i32; 3] = [4078, 4088, 2040];
/// Map id `WMBlock.Raycast` only accepts for control type 1 (`WMBlock.cs:165`).
pub const FLYING_ONLY_MAP_ID: i32 = 0x31EE;
/// Triangles whose up-facing normal component is at most this are skipped
/// (`WMPhysics.cs:20`).
pub const MIN_UP_NORMAL: f32 = 0.1;
/// `ff9.rayStartOffsetY` (`ff9/ff9.cs:1325`).
pub const RAY_START_OFFSET_Y: f32 = 2.34375;
/// `ff9.rayStartOffsetYFromSky` (`ff9/ff9.cs:1324`).
pub const RAY_START_OFFSET_Y_FROM_SKY: f32 = 400.0;
/// `ff9.defaultHeight` (`ff9/ff9.cs:1327`).
pub const DEFAULT_HEIGHT: f32 = 0.0;
/// Size of `s_moveCHRCache` arrays (`WMConstants.MovementCacheSize`).
pub const MOVE_CACHE_SIZE: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkMeshError {
    TangentCount {
        vertices: usize,
        tangents: usize,
    },
    IndexOutOfRange {
        index: u32,
        vertices: usize,
    },
    /// `WMBlock.AddWalkMesh` builds `vertices.Length / 3` triangle normals and would
    /// index past them when the mesh has more triangles.
    MoreTrianglesThanNormals {
        triangles: usize,
        normals: usize,
    },
}

impl std::fmt::Display for WalkMeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for WalkMeshError {}

/// One block mesh as `WMMesh` holds it (`WM/WMMesh.cs`), block-local vertices.
#[derive(Debug, Clone, PartialEq)]
pub struct WalkMesh {
    vertices: Vec<Vec3>,
    triangles: Vec<u32>,
    tangent_x: Vec<f32>,
    triangle_normals: Vec<Vec3>,
}

impl WalkMesh {
    /// Builds the mesh and its triangle normals (`WMBlock.AddWalkMesh`,
    /// `WMBlock.cs:42`): `normalize(cross(v1 - v0, v2 - v0))` per triangle.
    pub fn new(
        vertices: Vec<Vec3>,
        triangles: Vec<u32>,
        tangent_x: Vec<f32>,
    ) -> Result<Self, WalkMeshError> {
        if tangent_x.len() != vertices.len() {
            return Err(WalkMeshError::TangentCount {
                vertices: vertices.len(),
                tangents: tangent_x.len(),
            });
        }
        if let Some(&bad) = triangles.iter().find(|&&i| i as usize >= vertices.len()) {
            return Err(WalkMeshError::IndexOutOfRange {
                index: bad,
                vertices: vertices.len(),
            });
        }
        let count = triangles.len() / 3;
        let normal_count = vertices.len() / 3;
        if count > normal_count {
            return Err(WalkMeshError::MoreTrianglesThanNormals {
                triangles: count,
                normals: normal_count,
            });
        }
        let triangle_normals = (0..count)
            .map(|i| {
                let b = vertices[triangles[i * 3] as usize];
                let a = vertices[triangles[i * 3 + 1] as usize];
                let a2 = vertices[triangles[i * 3 + 2] as usize];
                (a - b).cross(a2 - b).normalized()
            })
            .collect();
        Ok(Self {
            vertices,
            triangles,
            tangent_x,
            triangle_normals,
        })
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len() / 3
    }

    /// `(int)tangents[triangles[i * 3]].x` (`WMBlock.cs:165`).
    pub fn map_id(&self, triangle: usize) -> i32 {
        self.tangent_x[self.triangles[triangle * 3] as usize] as i32
    }

    /// The block-local corners of a triangle.
    pub fn triangle_corners(&self, triangle: usize) -> [Vec3; 3] {
        [0, 1, 2].map(|corner| self.vertices[self.triangles[triangle * 3 + corner] as usize])
    }

    pub fn triangle_normal(&self, triangle: usize) -> Vec3 {
        self.triangle_normals[triangle]
    }

    fn corners(&self, triangle: usize, origin: Vec3) -> [Vec3; 3] {
        let at = |k: usize| self.vertices[self.triangles[triangle * 3 + k] as usize] + origin;
        [at(0), at(1), at(2)]
    }
}

/// `WMPhysics.intersect3D_RayTriangle` (`WMPhysics.cs:80`): `1` hit, `0` miss, `2`
/// coplanar ray, `-1` degenerate triangle. The hit point is zero unless it is `1`
/// or the barycentric tests were reached.
pub fn intersect_ray_triangle(origin: Vec3, direction: Vec3, t: [Vec3; 3]) -> (i32, Vec3) {
    let mut point = Vec3::ZERO;
    let v = t[1] - t[0];
    let w = t[2] - t[0];
    let normal = v.cross(w);
    if normal.unity_eq(Vec3::ZERO) {
        return (-1, point);
    }
    let rhs = origin - t[0];
    let num = -normal.dot(rhs);
    let num2 = normal.dot(direction);
    if num2.abs() < 1e-8 {
        return (if num == 0.0 { 2 } else { 0 }, point);
    }
    let num3 = num / num2;
    if num3 < 0.0 {
        return (0, point);
    }
    point = origin + num3 * direction;
    let num4 = v.dot(v);
    let num5 = v.dot(w);
    let num6 = w.dot(w);
    let lhs2 = point - t[0];
    let num7 = lhs2.dot(v);
    let num8 = lhs2.dot(w);
    let num9 = num5 * num5 - num4 * num6;
    let num10 = (num5 * num8 - num6 * num7) / num9;
    #[allow(clippy::manual_range_contains)]
    if num10 < 0.0 || num10 > 1.0 {
        return (0, point);
    }
    let num11 = (num5 * num7 - num4 * num8) / num9;
    if num11 < 0.0 || num10 + num11 > 1.0 {
        return (0, point);
    }
    (1, point)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    pub point: Vec3,
    pub triangle: usize,
}

/// `WMPhysics.Raycast` (`WMPhysics.cs:7`): first triangle in mesh order that is not
/// an excluded id, faces up, and is hit.
pub fn raycast_mesh(
    mesh: &WalkMesh,
    block_origin: Vec3,
    ray_origin: Vec3,
    direction: Vec3,
    ignore_exceptions: bool,
) -> Option<RayHit> {
    for triangle in 0..mesh.triangle_count() {
        let id = mesh.map_id(triangle);
        if !ignore_exceptions && EXCLUDED_MAP_IDS.contains(&id) {
            continue;
        }
        let up = Vec3::UP.dot(mesh.triangle_normal(triangle));
        if up <= MIN_UP_NORMAL && !ignore_exceptions {
            continue;
        }
        let corners = mesh.corners(triangle, block_origin);
        let (result, point) = intersect_ray_triangle(ray_origin, direction, corners);
        if result == 1 {
            return Some(RayHit { point, triangle });
        }
    }
    None
}

/// `WMPhysics.RaycastOnSpecifiedTriangle` (`WMPhysics.cs:52`): any non-zero
/// intersection result counts, including `-1` for a degenerate triangle.
pub fn raycast_triangle(
    mesh: &WalkMesh,
    triangle: usize,
    block_origin: Vec3,
    ray_origin: Vec3,
    direction: Vec3,
) -> Option<RayHit> {
    if triangle >= mesh.triangle_count() {
        return None;
    }
    let corners = mesh.corners(triangle, block_origin);
    let (result, point) = intersect_ray_triangle(ray_origin, direction, corners);
    (result != 0).then_some(RayHit { point, triangle })
}

/// Walk meshes of one block, in `WMWorld.LoadBlock` registration order
/// (`WMWorld.cs:508`): object, terrain, volcano, beach, stream, river, river joint,
/// falls, then the seas. Form 1 is the default form (`WMBlock._form`).
#[derive(Debug, Clone, PartialEq)]
pub struct BlockWalk {
    pub origin: Vec3,
    pub form: u8,
    pub form1: Vec<WalkMesh>,
    pub form2: Vec<WalkMesh>,
}

impl BlockWalk {
    /// `WMBlock.ActiveWalkMeshes` (`WMBlock.cs:11`); null for other forms.
    pub fn active(&self) -> &[WalkMesh] {
        match self.form {
            1 => &self.form1,
            2 => &self.form2,
            _ => &[],
        }
    }
}

/// `ff9.s_moveCHRCache` (`ff9/ff9.cs:10381`): the last ten triangles an actor stood
/// on, tried first on the next raycast so the result can differ from "first hit in
/// mesh order" where triangles overlap.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MoveCache {
    pub number: i32,
    pub on_object: [u8; MOVE_CACHE_SIZE],
    pub triangle_indices: [i32; MOVE_CACHE_SIZE],
    pub walk_mesh_indices: [i32; MOVE_CACHE_SIZE],
    pub blocks: [Option<(usize, usize)>; MOVE_CACHE_SIZE],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockHit {
    pub point: Vec3,
    pub triangle: i32,
    pub walk_mesh: i32,
    pub map_id: i32,
}

/// The static flags `WMPhysics` and `ff9.w_moveCHRControlPtr` supply to a raycast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RaycastMode {
    /// `WMPhysics.CastRayFromSky`.
    pub cast_from_sky: bool,
    /// `WMPhysics.IgnoreExceptions`.
    pub ignore_exceptions: bool,
    /// `ff9.w_moveCHRControlPtr.type` (1 = flying).
    pub control_type: u8,
}

fn accepts(map_id: i32, mode: &RaycastMode) -> bool {
    map_id != FLYING_ONLY_MAP_ID || mode.control_type == 1
}

/// `WMBlock.Raycast(ray, out hit, distance, out mapid, cache)` (`WMBlock.cs:112`).
/// The `distance` argument is unused in the source.
pub fn raycast_block(
    block: &BlockWalk,
    key: (usize, usize),
    ray_origin: Vec3,
    mode: &RaycastMode,
    cache: Option<&mut MoveCache>,
) -> Option<BlockHit> {
    let meshes = block.active();
    let direction = Vec3::DOWN;
    let mut found: Option<BlockHit> = None;
    let mut from_cache = false;
    if let Some(cache) = cache.as_deref() {
        for i in 0..MOVE_CACHE_SIZE as i32 {
            let slot = ((cache.number + i).rem_euclid(MOVE_CACHE_SIZE as i32)) as usize;
            if cache.blocks[slot] != Some(key) {
                continue;
            }
            let triangle = cache.triangle_indices[slot];
            let walk_mesh = cache.walk_mesh_indices[slot];
            let (Ok(triangle), Ok(walk_mesh_at)) =
                (usize::try_from(triangle), usize::try_from(walk_mesh))
            else {
                continue;
            };
            let Some(mesh) = meshes.get(walk_mesh_at) else {
                continue;
            };
            if let Some(hit) = raycast_triangle(mesh, triangle, block.origin, ray_origin, direction)
            {
                let map_id = mesh.map_id(hit.triangle);
                if accepts(map_id, mode) {
                    found = Some(BlockHit {
                        point: hit.point,
                        triangle: hit.triangle as i32,
                        walk_mesh,
                        map_id,
                    });
                    from_cache = true;
                    break;
                }
            }
        }
    }
    if found.is_none() {
        for (index, mesh) in meshes.iter().enumerate() {
            if let Some(hit) = raycast_mesh(
                mesh,
                block.origin,
                ray_origin,
                direction,
                mode.ignore_exceptions,
            ) {
                let map_id = mesh.map_id(hit.triangle);
                if accepts(map_id, mode) {
                    found = Some(BlockHit {
                        point: hit.point,
                        triangle: hit.triangle as i32,
                        walk_mesh: index as i32,
                        map_id,
                    });
                    break;
                }
            }
        }
    }
    let hit = found?;
    if let Some(cache) = cache
        && !from_cache
    {
        cache.number += 1;
        cache.number %= MOVE_CACHE_SIZE as i32;
        let slot = cache.number as usize;
        cache.triangle_indices[slot] = hit.triangle;
        cache.walk_mesh_indices[slot] = hit.walk_mesh;
        cache.blocks[slot] = Some(key);
        cache.on_object[slot] = 0;
    }
    Some(hit)
}

/// Result of `ff9.w_nwpHit` (`ff9/ff9.cs:6980`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NwpHit {
    /// Hit height, or `0` on a miss.
    pub height: f32,
    /// Full map id (event, area, topograph bits), `0` on a miss.
    pub id: i32,
    /// `1000` on a hit, `-1` on a miss.
    pub pno: i32,
    /// Block, walk-mesh index and triangle of the hit (not in the source's
    /// return value; kept for reports).
    pub block: Option<(usize, usize)>,
    pub walk_mesh: i32,
    pub triangle: i32,
}

/// All block walk meshes plus the block layout.
#[derive(Debug, Clone)]
pub struct WorldWalk {
    pub layout: BlockLayout,
    blocks: Vec<Option<BlockWalk>>,
}

impl Default for WorldWalk {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldWalk {
    pub fn new() -> Self {
        Self {
            layout: BlockLayout::identity(),
            blocks: vec![None; BLOCK_COLUMNS * BLOCK_ROWS],
        }
    }

    /// Installs a loaded block (`WMBlock.IsReady`).
    pub fn set_block(&mut self, column: usize, row: usize, block: BlockWalk) {
        self.blocks[column * BLOCK_ROWS + row] = Some(block);
    }

    pub fn block(&self, column: usize, row: usize) -> Option<&BlockWalk> {
        self.blocks.get(column * BLOCK_ROWS + row)?.as_ref()
    }

    /// `ff9.w_nwpHit` (`ff9/ff9.cs:6980`): casts a ray down from `pos.y + offset`
    /// into the block that holds `pos`. A missing or unloaded block gives
    /// `defaultHeight`, id 0 and `pno` -1.
    pub fn nwp_hit(&self, pos: Vec3, mode: &RaycastMode, cache: Option<&mut MoveCache>) -> NwpHit {
        let miss = NwpHit {
            height: DEFAULT_HEIGHT,
            id: 0,
            pno: -1,
            block: None,
            walk_mesh: -1,
            triangle: -1,
        };
        let Some(key) = self.layout.absolute_block(pos) else {
            return miss;
        };
        let Some(block) = self.block(key.0, key.1) else {
            return miss;
        };
        let mut origin = pos;
        origin.y += if mode.cast_from_sky {
            RAY_START_OFFSET_Y_FROM_SKY
        } else {
            RAY_START_OFFSET_Y
        };
        match raycast_block(block, key, origin, mode, cache) {
            Some(hit) => NwpHit {
                height: hit.point.y,
                id: hit.map_id,
                pno: 1000,
                block: Some(key),
                walk_mesh: hit.walk_mesh,
                triangle: hit.triangle,
            },
            None => NwpHit {
                block: Some(key),
                ..miss
            },
        }
    }
}
