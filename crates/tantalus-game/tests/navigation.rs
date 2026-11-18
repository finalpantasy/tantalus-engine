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
        actor_anchor: point(2, 7, 2),
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

#[test]
fn snapping_takes_the_triangle_nearest_in_height_or_the_default_center() {
    let mesh = mesh();
    let navigation = FieldNavigation::new(&mesh).unwrap();
    // Height 32768 takes the highest triangle holding the point (floor 0
    // here; floor 1 sits lower once the vertex height is negated).
    let high = navigation.snap_to_walkmesh(point(10, 32768, 5)).unwrap();
    assert!(high.triangle_index < 2);
    assert_eq!((high.position.x, high.position.z), (10, 5));
    // A height near floor 1 takes floor 1.
    let low = navigation.snap_to_walkmesh(point(10, -110, 5)).unwrap();
    assert!(low.triangle_index >= 2);
    // Off every triangle: center of triangle 0 (`SetDefaultCharPos`).
    let off = navigation.snap_to_walkmesh(point(500, 0, 500)).unwrap();
    assert_eq!(off.triangle_index, 0);
    assert_eq!((off.position.x, off.position.z), (13, 6));
}

/// A hall floor (world height 0, z 0..200) under a gallery (height 200,
/// x 0..50, z 0..100) whose x = 50 edge links to a ramp down to height 100
/// at x = 100, the shape of map 1823's spiral ramp over the hall.
fn gallery_over_hall() -> FieldNavigationMesh {
    let triangle = |vertices: [u32; 3], floor| NavigationTriangle {
        vertices,
        floor,
        offset: Point3::default(),
    };
    FieldNavigationMesh {
        vertices: vec![
            point(0, 0, 0),
            point(100, 0, 0),
            point(100, 0, 200),
            point(0, 0, 200),
            point(0, -200, 0),
            point(50, -200, 0),
            point(50, -200, 100),
            point(0, -200, 100),
            point(100, -100, 0),
            point(100, -100, 100),
        ],
        triangles: vec![
            triangle([0, 1, 2], 0),
            triangle([0, 2, 3], 0),
            triangle([4, 5, 6], 1),
            triangle([4, 6, 7], 1),
            triangle([5, 8, 9], 2),
            triangle([5, 9, 6], 2),
        ],
        active_floor: 0,
        active_triangle: 0,
        actor_anchor: point(0, 0, 0),
    }
}

#[test]
fn a_step_crosses_linked_edges_and_never_drops_to_a_floor_below() {
    let mesh = gallery_over_hall();
    let navigation = FieldNavigation::new(&mesh).unwrap();
    // Off the gallery's unlinked z = 100 edge, over the hall only: the step
    // stops on the gallery edge at its height (`BGI_traverseTriangles`
    // finds no neighbor), where the any-floor clamp drops to the hall.
    let blocked = navigation
        .step_along(point(25, 200, 90), point(25, 200, 150))
        .unwrap();
    assert!(blocked.triangle_index == 2 || blocked.triangle_index == 3);
    assert_eq!(blocked.position.y, 200);
    assert!((98..=100).contains(&blocked.position.z), "{blocked:?}");
    let dropped = navigation.nearest_on_walkmesh(point(25, 200, 150)).unwrap();
    assert_eq!(dropped.position.y, 0);
    // Across the linked x = 50 edge: onto the ramp, not the hall below.
    let ramp = navigation
        .step_along(point(40, 200, 50), point(75, 200, 50))
        .unwrap();
    assert!(ramp.triangle_index >= 4, "{ramp:?}");
    assert_eq!(ramp.position, point(75, 150, 50));
    // Inside the start triangle: the point at the triangle's height.
    let inside = navigation
        .step_along(point(10, 200, 50), point(20, 0, 60))
        .unwrap();
    assert_eq!(inside.position, point(20, 200, 60));
}
