//! Field camera projection: where a field-world point lands on the
//! background, from `PSX.CalculateGTE_RTPT_POS` with a `BGCAM_DEF` camera
//! (`GetMatrixRT`: rotation in 1/4096 units plus translation, `proj` as the
//! view distance, `centerOffset` added after the divide).

use bevy::prelude::*;
use tantalus_contracts::{FieldNavigationMesh, LegacyFieldCamera, Point3};

/// Where a field-world point lands in the 2D space the scene adapter draws
/// the background in (x right, y up, placed by [`projection_offset`]), or
/// `None` when the point sits on the camera plane.
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
    let offset = projection_offset(camera);
    Some([
        out[0] * view / depth + offset[0],
        out[1] * view / depth + offset[1],
    ])
}

/// Half the PSX field frame (`FieldMap.HalfFieldWidth`/`HalfFieldHeight`).
pub const HALF_FIELD: [f32; 2] = [160.0, 112.0];

/// `FieldMap.offset`, added to every projected point
/// (`FieldMap.cs:393-395`): `centerOffset.x + w / 2 - HalfFieldWidth` and
/// `-centerOffset.y - h / 2 + HalfFieldHeight`.
pub fn projection_offset(camera: &LegacyFieldCamera) -> [f32; 2] {
    [
        f32::from(camera.center_offset[0]) + f32::from(camera.size[0]) / 2.0 - HALF_FIELD[0],
        -f32::from(camera.center_offset[1]) - f32::from(camera.size[1]) / 2.0 + HALF_FIELD[1],
    ]
}

/// The camera's scroll range in the space the adapter draws in: the
/// viewport (`vrpMinX..vrpMaxX`, `vrpMinY..vrpMaxY`, background pixels
/// from the top left, naming the frame's center) as `[min x, max x, min y,
/// max y]` with y up. An all-zero viewport is read as the frame centered
/// on the background.
pub fn view_range(camera: &LegacyFieldCamera) -> [f32; 4] {
    let (vx, vy) = if camera.viewport_x == [0, 0] && camera.viewport_y == [0, 0] {
        ([HALF_FIELD[0]; 2], [HALF_FIELD[1]; 2])
    } else {
        (
            camera.viewport_x.map(f32::from),
            camera.viewport_y.map(f32::from),
        )
    };
    let x = [
        vx[0].min(vx[1]) - HALF_FIELD[0],
        vx[0].max(vx[1]) - HALF_FIELD[0],
    ];
    let y = [
        HALF_FIELD[1] - vy[0].max(vy[1]),
        HALF_FIELD[1] - vy[0].min(vy[1]),
    ];
    [x[0], x[1], y[0], y[1]]
}

/// Where the camera looks to follow `target`, kept in [`view_range`].
pub fn follow(camera: &LegacyFieldCamera, target: [f32; 2]) -> [f32; 2] {
    let range = view_range(camera);
    [
        target[0].clamp(range[0], range[1]),
        target[1].clamp(range[2], range[3]),
    ]
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
    fn view_range_follows_the_viewport() {
        // 160..208 and 112..144 in background pixels.
        assert_eq!(view_range(&camera()), [0.0, 48.0, -32.0, 0.0]);
        assert_eq!(follow(&camera(), [100.0, 100.0]), [48.0, 0.0]);
        let unset = LegacyFieldCamera {
            viewport_x: [0, 0],
            viewport_y: [0, 0],
            ..camera()
        };
        assert_eq!(view_range(&unset), [0.0; 4]);
    }

    #[test]
    fn nearer_floor_points_project_lower_on_screen() {
        let far = project_field_point(&camera(), [2920.0, 0.0, 12266.0]).unwrap();
        let near = project_field_point(&camera(), [1787.0, 0.0, 3261.0]).unwrap();
        // Offset (0 + 184 - 160, -0 - 128 + 112) = (24, -16).
        assert!(
            (far[0] - 91.9).abs() < 0.1 && (far[1] + 51.86).abs() < 0.1,
            "{far:?}"
        );
        assert!(
            (near[0] - 258.2).abs() < 0.1 && (near[1] + 189.9).abs() < 0.1,
            "{near:?}"
        );
        assert!(near[1] < far[1]);
    }
}
