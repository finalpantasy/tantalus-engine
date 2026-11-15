//! Pose math: 4x4 matrices, legacy-curve sampling, clip-to-skeleton binding
//! and CPU skinning. Matrices are row-major `e{row}{col}` (Unity's
//! `Matrix4x4` field order, as stored in `Mesh.m_BindPose`) acting on column
//! vectors.

use super::{AssembledModel, ModelNode, SkinnedPart};
use crate::pc::unity_object::animation::{Curve, Keyframe};
use crate::pc::unity_object::{Aabb, AnimationClip};
use std::collections::HashMap;

pub type Mat4 = [f32; 16];

pub const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// Local transform of one node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodePose {
    pub position: [f32; 3],
    /// Quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl NodePose {
    pub fn of(node: &ModelNode) -> Self {
        Self {
            position: node.local_position,
            rotation: node.local_rotation,
            scale: node.local_scale,
        }
    }
}

pub fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0; 16];
    for row in 0..4 {
        for col in 0..4 {
            out[row * 4 + col] = (0..4).map(|k| a[row * 4 + k] * b[k * 4 + col]).sum();
        }
    }
    out
}

pub fn normalize_quat(q: [f32; 4]) -> [f32; 4] {
    let length = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if length > 1e-12 && length.is_finite() {
        [q[0] / length, q[1] / length, q[2] / length, q[3] / length]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// `T * R * S` for a local pose (rotation is normalized first).
pub fn mat_from_pose(pose: &NodePose) -> Mat4 {
    let [x, y, z, w] = normalize_quat(pose.rotation);
    let [sx, sy, sz] = pose.scale;
    let [px, py, pz] = pose.position;
    [
        (1.0 - 2.0 * (y * y + z * z)) * sx,
        2.0 * (x * y - z * w) * sy,
        2.0 * (x * z + y * w) * sz,
        px,
        2.0 * (x * y + z * w) * sx,
        (1.0 - 2.0 * (x * x + z * z)) * sy,
        2.0 * (y * z - x * w) * sz,
        py,
        2.0 * (x * z - y * w) * sx,
        2.0 * (y * z + x * w) * sy,
        (1.0 - 2.0 * (x * x + y * y)) * sz,
        pz,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

/// Inverse of an affine matrix (last row `0 0 0 1`); `None` when singular.
pub fn mat_affine_inverse(m: &Mat4) -> Option<Mat4> {
    let a = |r: usize, c: usize| f64::from(m[r * 4 + c]);
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1))
        - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    if det.abs() < 1e-18 || !det.is_finite() {
        return None;
    }
    let inv = [
        [
            (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1)) / det,
            (a(0, 2) * a(2, 1) - a(0, 1) * a(2, 2)) / det,
            (a(0, 1) * a(1, 2) - a(0, 2) * a(1, 1)) / det,
        ],
        [
            (a(1, 2) * a(2, 0) - a(1, 0) * a(2, 2)) / det,
            (a(0, 0) * a(2, 2) - a(0, 2) * a(2, 0)) / det,
            (a(0, 2) * a(1, 0) - a(0, 0) * a(1, 2)) / det,
        ],
        [
            (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0)) / det,
            (a(0, 1) * a(2, 0) - a(0, 0) * a(2, 1)) / det,
            (a(0, 0) * a(1, 1) - a(0, 1) * a(1, 0)) / det,
        ],
    ];
    let t = [a(0, 3), a(1, 3), a(2, 3)];
    let mut out = IDENTITY;
    for row in 0..3 {
        for col in 0..3 {
            out[row * 4 + col] = inv[row][col] as f32;
        }
        out[row * 4 + 3] = -(inv[row][0] * t[0] + inv[row][1] * t[1] + inv[row][2] * t[2]) as f32;
    }
    Some(out)
}

pub fn transform_point(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
    [
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
        m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
        m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
    ]
}

/// Rest-pose local transforms of every node.
pub fn rest_pose(model: &AssembledModel) -> Vec<NodePose> {
    model.nodes.iter().map(NodePose::of).collect()
}

/// World matrices for `poses`. Nodes are stored parent-before-child.
pub fn world_matrices(nodes: &[ModelNode], poses: &[NodePose]) -> Vec<Mat4> {
    let mut world: Vec<Mat4> = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let local = mat_from_pose(&poses[index]);
        world.push(match node.parent {
            Some(parent) => mat_mul(&world[parent], &local),
            None => local,
        });
    }
    world
}

/// Bounds of a point set as center/extent; `None` when empty.
pub fn bounds_of(points: &[[f32; 3]]) -> Option<Aabb> {
    let first = *points.first()?;
    let (mut min, mut max) = (first, first);
    for point in points {
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    Some(Aabb {
        center: [0, 1, 2].map(|a| (min[a] + max[a]) * 0.5),
        extent: [0, 1, 2].map(|a| (max[a] - min[a]) * 0.5),
    })
}

/// Union of two bounds.
pub fn union_bounds(a: Aabb, b: Aabb) -> Aabb {
    let mut min = [0.0; 3];
    let mut max = [0.0; 3];
    for axis in 0..3 {
        min[axis] = (a.center[axis] - a.extent[axis]).min(b.center[axis] - b.extent[axis]);
        max[axis] = (a.center[axis] + a.extent[axis]).max(b.center[axis] + b.extent[axis]);
    }
    Aabb {
        center: [0, 1, 2].map(|i| (min[i] + max[i]) * 0.5),
        extent: [0, 1, 2].map(|i| (max[i] - min[i]) * 0.5),
    }
}

/// Per-bone skinning matrices `boneWorld * bindPose`, giving posed
/// positions in the same space as the node world matrices (Unity ignores
/// the renderer's own transform when skinning). A bone that failed to
/// resolve yields `None`.
pub fn skinning_matrices(part: &SkinnedPart, world: &[Mat4]) -> Vec<Option<Mat4>> {
    part.bones
        .iter()
        .zip(&part.bind_poses)
        .map(|(bone, bind)| bone.map(|node| mat_mul(&world[node], bind)))
        .collect()
}

/// CPU linear-blend skinning. Weights of unresolved bones are dropped and
/// the remaining weights are used as stored (not renormalized); a vertex
/// with no usable influence keeps its mesh-local position.
pub fn skin_vertices(part: &SkinnedPart, matrices: &[Option<Mat4>]) -> Vec<[f32; 3]> {
    part.positions
        .iter()
        .enumerate()
        .map(|(vertex, position)| {
            let Some(influence) = part.skin.get(vertex) else {
                return *position;
            };
            let mut out = [0.0_f32; 3];
            let mut used = false;
            for slot in 0..4 {
                let weight = influence.weights[slot];
                if weight == 0.0 {
                    continue;
                }
                let Some(Some(matrix)) = usize::try_from(influence.bones[slot])
                    .ok()
                    .and_then(|bone| matrices.get(bone))
                else {
                    continue;
                };
                let moved = transform_point(matrix, *position);
                for axis in 0..3 {
                    out[axis] += moved[axis] * weight;
                }
                used = true;
            }
            if used { out } else { *position }
        })
        .collect()
}

/// Skinned positions of every part for `poses`, in the model's world space.
pub fn skin_model(model: &AssembledModel, poses: &[NodePose]) -> Vec<Vec<[f32; 3]>> {
    let world = world_matrices(&model.nodes, poses);
    model
        .parts
        .iter()
        .map(|part| skin_vertices(part, &skinning_matrices(part, &world)))
        .collect()
}

/// Bounds of all skinned parts together.
pub fn skinned_bounds(model: &AssembledModel, poses: &[NodePose]) -> Option<Aabb> {
    skin_model(model, poses)
        .iter()
        .filter_map(|points| bounds_of(points))
        .reduce(union_bounds)
}

/// How legacy keys are interpolated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    /// Straight lerp between neighboring keys, ignoring tangents.
    Linear,
    /// Cubic Hermite from the stored in/out slopes (Unity's legacy curve
    /// evaluation). An infinite slope makes the segment stepped.
    Hermite,
}

/// Unity curve `WrapMode` values used for pre/post infinity.
const WRAP_LOOP: i32 = 2;
const WRAP_PING_PONG: i32 = 4;

/// Maps `time` into the key range according to the curve's pre/post
/// infinity: loop and ping-pong repeat the range, anything else clamps.
pub fn wrap_time<const N: usize>(
    keys: &[Keyframe<N>],
    time: f32,
    pre_infinity: i32,
    post_infinity: i32,
) -> f32 {
    let (Some(first), Some(last)) = (keys.first(), keys.last()) else {
        return time;
    };
    let span = last.time - first.time;
    if span <= 0.0 || !time.is_finite() {
        return time;
    }
    let mode = if time < first.time {
        pre_infinity
    } else if time > last.time {
        post_infinity
    } else {
        return time;
    };
    let offset = time - first.time;
    match mode {
        WRAP_LOOP => first.time + offset.rem_euclid(span),
        WRAP_PING_PONG => {
            let phase = offset.rem_euclid(span * 2.0);
            first.time
                + if phase > span {
                    span * 2.0 - phase
                } else {
                    phase
                }
        }
        _ => time,
    }
}

/// Samples a whole curve, honoring its pre/post infinity.
pub fn sample_full_curve<const N: usize>(
    curve: &Curve<N>,
    time: f32,
    mode: Interpolation,
) -> Option<[f32; N]> {
    let time = wrap_time(&curve.keys, time, curve.pre_infinity, curve.post_infinity);
    sample_curve(&curve.keys, time, mode)
}

/// Evaluates one curve at `time`, clamping outside the key range. `None`
/// when the curve has no keys.
pub fn sample_curve<const N: usize>(
    keys: &[Keyframe<N>],
    time: f32,
    mode: Interpolation,
) -> Option<[f32; N]> {
    let first = keys.first()?;
    let last = keys.last()?;
    if time <= first.time || keys.len() == 1 {
        return Some(first.value);
    }
    if time >= last.time {
        return Some(last.value);
    }
    let upper = keys.partition_point(|key| key.time <= time);
    let (a, b) = (&keys[upper - 1], &keys[upper]);
    let dt = b.time - a.time;
    if dt <= 0.0 {
        return Some(b.value);
    }
    let u = (time - a.time) / dt;
    let mut out = [0.0; N];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = match mode {
            Interpolation::Linear => a.value[index] + (b.value[index] - a.value[index]) * u,
            Interpolation::Hermite => {
                let (m0, m1) = (a.out_slope[index], b.in_slope[index]);
                if !m0.is_finite() || !m1.is_finite() {
                    a.value[index]
                } else {
                    let (u2, u3) = (u * u, u * u * u);
                    (2.0 * u3 - 3.0 * u2 + 1.0) * a.value[index]
                        + (u3 - 2.0 * u2 + u) * dt * m0
                        + (-2.0 * u3 + 3.0 * u2) * b.value[index]
                        + (u3 - u2) * dt * m1
                }
            }
        };
    }
    Some(out)
}

/// Result of matching a clip's curve paths to a model's node paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipBinding {
    /// Node index per curve, in clip order (`None` = no node at that path).
    pub rotation: Vec<Option<usize>>,
    pub position: Vec<Option<usize>>,
    pub scale: Vec<Option<usize>>,
    /// Transform curves (rotation + position + scale) that found a node.
    pub matched: usize,
    pub unmatched: usize,
    /// Float curves drive component properties, not transforms; they are
    /// counted but never matched to nodes.
    pub float_curves: usize,
    pub unmatched_paths: Vec<String>,
}

impl ClipBinding {
    /// All transform curves found a node.
    pub fn fully_matched(&self) -> bool {
        self.unmatched == 0
    }
}

/// Matches a node-path lookup against the curves. Paths are relative to
/// the animated root (`""` is the root itself); the first node with a given
/// path wins, as in Unity.
pub fn bind_clip_paths(paths: &HashMap<&str, usize>, clip: &AnimationClip) -> ClipBinding {
    let mut matched = 0;
    let mut unmatched = 0;
    let mut unmatched_paths: Vec<String> = Vec::new();
    let mut resolve = |path: &str| {
        let node = paths.get(path).copied();
        if node.is_some() {
            matched += 1;
        } else {
            unmatched += 1;
            if unmatched_paths.len() < 8 && !unmatched_paths.iter().any(|seen| seen == path) {
                unmatched_paths.push(path.to_owned());
            }
        }
        node
    };
    fn run<const N: usize>(
        curves: &[Curve<N>],
        resolve: &mut impl FnMut(&str) -> Option<usize>,
    ) -> Vec<Option<usize>> {
        curves.iter().map(|curve| resolve(&curve.path)).collect()
    }
    let rotation = run(&clip.rotation_curves, &mut resolve);
    let position = run(&clip.position_curves, &mut resolve);
    let scale = run(&clip.scale_curves, &mut resolve);
    ClipBinding {
        rotation,
        position,
        scale,
        matched,
        unmatched,
        float_curves: clip.float_curves.len(),
        unmatched_paths,
    }
}

/// Binds `clip` to `model`.
pub fn bind_clip(model: &AssembledModel, clip: &AnimationClip) -> ClipBinding {
    bind_clip_paths(&model.path_lookup(), clip)
}

/// Samples local TRS for every node at `time` seconds: rest pose with each
/// bound curve's channel overridden. Rotations are normalized after
/// interpolation.
pub fn sample_clip(
    model: &AssembledModel,
    clip: &AnimationClip,
    binding: &ClipBinding,
    time: f32,
    mode: Interpolation,
) -> Vec<NodePose> {
    let mut poses = rest_pose(model);
    for (curve, node) in clip.rotation_curves.iter().zip(&binding.rotation) {
        if let (Some(node), Some(value)) = (node, sample_full_curve(curve, time, mode)) {
            poses[*node].rotation = normalize_quat(value);
        }
    }
    for (curve, node) in clip.position_curves.iter().zip(&binding.position) {
        if let (Some(node), Some(value)) = (node, sample_full_curve(curve, time, mode)) {
            poses[*node].position = value;
        }
    }
    for (curve, node) in clip.scale_curves.iter().zip(&binding.scale) {
        if let (Some(node), Some(value)) = (node, sample_full_curve(curve, time, mode)) {
            poses[*node].scale = value;
        }
    }
    poses
}
