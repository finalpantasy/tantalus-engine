//! Field camera projection: where a field-world point lands on the
//! background, from `PSX.CalculateGTE_RTPT_POS` with a `BGCAM_DEF` camera
//! (`GetMatrixRT`: rotation in 1/4096 units plus translation, `proj` as the
//! view distance, `centerOffset` added after the divide).

use bevy::prelude::*;
use tantalus_contracts::{FieldNavigationMesh, LegacyFieldCamera, Point3};

/// Where a field-world point lands in the 2D space the scene adapter draws
/// the background in (x right, y up; the field camera looks at
/// `center_offset`), or `None` when the point sits on the camera plane.
/// The GTE result is already y-up here: `RTPT_POS` flips y on the way in
/// and back out.
pub fn project_field_point(camera: &LegacyFieldCamera, point: [f32; 3]) -> Option<[f32; 2]> {
    let r = |row: usize, column: usize| f32::from(camera.rotation[row * 3 + column]) / 4096.0;
    let t = [
        camera.translation.x as f32,
        camera.translation.y as f32,
        camera.translation.z as f32,
    ];
    // `v.y *= -1` before the camera transform.
    let v = [point[0], -point[1], point[2]];
    let mut out = [0.0f32; 3];
    for (row, value) in out.iter_mut().enumerate() {
        *value = r(row, 0) * v[0] + r(row, 1) * v[1] + r(row, 2) * v[2] + t[row];
    }
    out[1] = -out[1];
    let depth = out[2].abs();
    if depth < f32::EPSILON {
        return None;
    }
    let view = f32::from(camera.projection);
    Some([
        out[0] * view / depth + f32::from(camera.center_offset[0]),
        out[1] * view / depth + f32::from(camera.center_offset[1]),
    ])
}

/// World-space corners of each walkmesh triangle (vertex plus the
/// triangle's floor offset, as `tantalus_game::navigation` builds them).
pub fn walkmesh_points(mesh: &FieldNavigationMesh) -> Vec<[f32; 3]> {
    let mut points = Vec::new();
    for triangle in &mesh.triangles {
        for index in triangle.vertices {
            let Some(vertex) = mesh.vertices.get(index as usize) else {
                continue;
            };
            let offset: Point3 = triangle.offset;
            points.push([
                (vertex.x + offset.x) as f32,
                -((vertex.y + offset.y) as f32),
                (vertex.z + offset.z) as f32,
            ]);
        }
    }
    points
}

/// Marks each projected walkmesh corner with a small dot above the
/// background; a debug view of how the camera maps the floor.
pub fn spawn_walkmesh_dots(
    world: &mut World,
    camera: &LegacyFieldCamera,
    mesh: &FieldNavigationMesh,
) -> usize {
    let mut count = 0;
    for point in walkmesh_points(mesh) {
        if let Some([x, y]) = project_field_point(camera, point) {
            world.spawn((
                Sprite::from_color(Color::srgb(1.0, 0.2, 0.8), Vec2::splat(3.0)),
                Transform::from_xyz(x, y, 450.0),
            ));
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> LegacyFieldCamera {
        // EF_TNT (map 262) camera.
        LegacyFieldCamera {
            projection: 335,
            rotation: [4096, 43, 2, -40, 3818, 184, 0, -197, 4091],
            translation: Point3 {
                x: -823,
                y: 588,
                z: -1876,
            },
            center_offset: [0, 0],
            size: [368, 256],
            viewport_x: [160, 208],
            viewport_y: [112, 144],
            depth_offset: 0,
        }
    }

    #[test]
    fn nearer_floor_points_project_lower_on_screen() {
        let far = project_field_point(&camera(), [2920.0, 0.0, 12266.0]).unwrap();
        let near = project_field_point(&camera(), [1787.0, 0.0, 3261.0]).unwrap();
        assert!(
            (far[0] - 67.9).abs() < 0.1 && (far[1] + 35.86).abs() < 0.1,
            "{far:?}"
        );
        assert!(
            (near[0] - 234.2).abs() < 0.1 && (near[1] + 173.9).abs() < 0.1,
            "{near:?}"
        );
        assert!(near[1] < far[1]);
    }
}
