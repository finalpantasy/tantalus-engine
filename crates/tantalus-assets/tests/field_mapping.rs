use tantalus_assets::pc::bgi::*;
use tantalus_assets::pc::bgs::*;
use tantalus_assets::pc::field::FieldMapper;
use tantalus_contracts::*;

fn scene(depth_shift: u16) -> BgsFile {
    BgsFile {
        scene_length: 0,
        depth_bit_shift: depth_shift,
        origin_z: 0,
        current_z: 0,
        origin: [0; 2],
        current: [0; 2],
        limits: [0; 4],
        screen: [0; 2],
        animations: Vec::new(),
        overlays: vec![BgsOverlay {
            flags: 3,
            current_z: 4,
            original_z: 2,
            width: 16,
            height: 8,
            origin: [0; 2],
            current: [20, 30],
            limits: [0; 4],
            screen: [0; 2],
            scroll: [0; 2],
            fraction: [0; 2],
            camera: 0,
            is_x_offset: false,
            viewport: 0,
            sprites: vec![BgsSprite {
                clut_y: 0,
                clut_x: 0,
                texture_y: 0,
                texture_x: 0,
                resolution: 0,
                alpha: 2,
                v: 50,
                u: 40,
                height: 8,
                width: 16,
                transparency: 1,
                depth: 7,
                offset_y: 2,
                offset_x: 3,
            }],
        }],
        lights: Vec::new(),
        cameras: vec![BgsCamera {
            projection: 529,
            rotation: [[4096, 0, 0], [0, 4096, 0], [0, 0, 4096]],
            translation: [10, 20, 30],
            center_offset: [1, 2],
            width: 320,
            height: 224,
            viewport_x: [-160, 160],
            viewport_y: [-112, 112],
            depth_offset: 9,
        }],
    }
}

fn navigation() -> BgiFile {
    BgiFile {
        origin: [0; 3],
        current: [0; 3],
        minimum: [0; 3],
        maximum: [0; 3],
        character: [100, 5, 200],
        active_floor: 0,
        active_triangle: 0,
        triangles: vec![BgiTriangle {
            flags: 0,
            data: 0,
            floor: 0,
            normal: 0,
            theta_x: 0,
            theta_z: 0,
            vertices: [0, 1, 2],
            edges: [0; 3],
            neighbors: [-1; 3],
            center: [0; 3],
            d: 0,
        }],
        edges: Vec::new(),
        animations: Vec::new(),
        floors: vec![BgiFloor {
            flags: 0,
            index: 0,
            bounds: [[0; 3]; 4],
            triangles: vec![0],
        }],
        normals: Vec::new(),
        vertices: Vec::new(),
    }
}

#[test]
fn field_mapping_preserves_atlas_offsets_depth_camera_and_navigation_anchor() {
    let mapped = FieldMapper::map(
        &scene(2),
        &navigation(),
        ContentId::parse("fixture:texture/atlas").unwrap(),
        0,
    )
    .unwrap();
    assert_eq!(mapped.camera.projection, 529);
    assert_eq!(
        mapped.camera.translation,
        Point3 {
            x: 10,
            y: 20,
            z: 30
        }
    );
    assert_eq!(
        mapped.actor_anchor,
        Point3 {
            x: 100,
            y: 5,
            z: 200
        }
    );
    assert_eq!(mapped.navigation.triangles.len(), 1);
    assert_eq!(mapped.navigation.active_floor, 0);
    let sprite = &mapped.overlays[0].sprites[0];
    assert_eq!(sprite.atlas_rect, [40, 50, 56, 58]);
    assert_eq!(sprite.pixel_offset, [-137, -80]);
    // scene 0 + overlay 4 + sprite 7; the depth shift does not apply.
    assert_eq!(sprite.depth, 11);
    assert!(sprite.active && sprite.screen_anchored);
}

#[test]
fn invalid_camera_and_negative_depth_are_rejected() {
    assert!(
        FieldMapper::map(
            &scene(0),
            &navigation(),
            ContentId::parse("fixture:texture/atlas").unwrap(),
            1
        )
        .is_err()
    );
    let mut behind = scene(2);
    behind.current_z = -100;
    assert!(
        FieldMapper::map(
            &behind,
            &navigation(),
            ContentId::parse("fixture:texture/atlas").unwrap(),
            0
        )
        .is_err()
    );
}

#[test]
fn upscaled_atlas_uses_sequential_padded_tiles_and_native_sprite_sizes() {
    let mut scene = scene(2);
    scene.overlays.push(scene.overlays[0].clone());
    let mapped = FieldMapper::map_upscaled(
        &scene,
        &navigation(),
        ContentId::parse("fixture:texture/atlas").unwrap(),
        0,
        [128, 128],
        32,
        2,
    )
    .unwrap();

    let first = &mapped.overlays[0].sprites[0];
    let second = &mapped.overlays[1].sprites[0];
    assert_eq!(first.atlas_rect, [2, 2, 34, 34]);
    assert_eq!(second.atlas_rect, [38, 2, 70, 34]);
    assert_eq!(first.pixel_size, [16, 8]);
    assert_eq!(second.pixel_offset, first.pixel_offset);

    assert!(
        FieldMapper::map_upscaled(
            &scene,
            &navigation(),
            ContentId::parse("fixture:texture/atlas").unwrap(),
            0,
            [32, 32],
            32,
            2,
        )
        .is_err()
    );
}
