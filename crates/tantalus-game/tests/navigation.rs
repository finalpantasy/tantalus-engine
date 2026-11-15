use tantalus_contracts::{FieldNavigationMesh, NavigationTriangle, Point3};
use tantalus_game::navigation::FieldNavigation;

fn point(x: i32, y: i32, z: i32) -> Point3 {
    Point3 { x, y, z }
}

fn mesh() -> FieldNavigationMesh {
    FieldNavigationMesh {
        vertices: vec![
            point(0, 0, 0),
            point(20, 0, 0),
            point(20, 20, 20),
            point(0, 20, 20),
            point(0, 100, 0),
            point(20, 100, 0),
            point(20, 120, 20),
            point(0, 120, 20),
        ],
        triangles: vec![
            NavigationTriangle {
                vertices: [0, 1, 2],
                floor: 0,
                offset: Point3::default(),
            },
            NavigationTriangle {
                vertices: [0, 2, 3],
                floor: 0,
                offset: Point3::default(),
            },
            NavigationTriangle {
                vertices: [4, 5, 6],
                floor: 1,
                offset: Point3::default(),
            },
            NavigationTriangle {
                vertices: [4, 6, 7],
                floor: 1,
                offset: Point3::default(),
            },
        ],
        active_floor: 0,
        active_triangle: 0,
    }
}

#[test]
fn projection_interpolates_height_and_movement_crosses_triangles() {
    let mesh = mesh();
    let navigation = FieldNavigation::new(&mesh).unwrap();
    let anchor = navigation.project(point(2, 7, 2)).unwrap();
    assert_eq!(anchor.position, point(2, -2, 2));
    assert_eq!(anchor.triangle_index, 0);

    let moved = navigation.move_by(anchor.position, [10, 14]).unwrap();
    assert_eq!(moved.position, point(12, -16, 16));
    assert_eq!(moved.triangle_index, 1);
}

#[test]
fn movement_clamps_to_the_nearest_walkable_edge() {
    let mesh = mesh();
    let navigation = FieldNavigation::new(&mesh).unwrap();
    let moved = navigation.move_by(point(18, -18, 18), [20, 0]).unwrap();
    assert_eq!(moved.position, point(20, -18, 18));
    assert_eq!(moved.triangle_index, 0);
}

#[test]
fn explicit_floor_selection_changes_the_height_surface() {
    let mesh = mesh();
    let navigation = FieldNavigation::new(&mesh).unwrap();
    let projected = navigation.project_on_floor(point(12, 0, 16), 1).unwrap();
    assert_eq!(projected.position, point(12, -116, 16));
    assert_eq!(projected.triangle_index, 3);
}

#[test]
fn invalid_indices_and_empty_floors_are_specific_errors() {
    let mut invalid = mesh();
    invalid.triangles[0].vertices[0] = 99;
    assert!(FieldNavigation::new(&invalid).is_err());

    let mesh = mesh();
    let navigation = FieldNavigation::new(&mesh).unwrap();
    let error = navigation.project_on_floor(point(0, 0, 0), 9);
    assert!(error.unwrap_err().context.contains("floor 9"));
}

#[test]
fn floor_origin_offsets_apply_before_height_inversion() {
    let mut mesh = mesh();
    for triangle in &mut mesh.triangles[..2] {
        triangle.offset = point(5, 7, 9);
    }
    assert_eq!(mesh.triangles[1].offset, point(5, 7, 9));
    let navigation = FieldNavigation::new(&mesh).unwrap();
    let projected = navigation.project(point(12, 0, 16)).unwrap();
    assert_eq!(projected.position, point(12, -14, 16));
    assert_eq!(projected.triangle_index, 0);
}
