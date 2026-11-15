//! World-map movement, collision and camera tests. All meshes are project-owned
//! synthetic quads; every expected value is derived by hand from the Memoria source
//! cited on the ported function.

use tantalus_legacy::world::*;

const EPS: f32 = 1e-4;

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < EPS
}

/// Topograph `t` with no area or event bits.
const fn topo_id(t: i32) -> i32 {
    t << 2
}

#[derive(Default)]
struct MeshBuilder {
    vertices: Vec<Vec3>,
    indices: Vec<u32>,
    tangents: Vec<f32>,
}

impl MeshBuilder {
    /// One up-facing triangle with its own three vertices (the shipped meshes keep
    /// `vertices == 3 * triangles`).
    fn triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, id: i32) {
        for v in [a, b, c] {
            self.indices.push(self.vertices.len() as u32);
            self.vertices.push(v);
            self.tangents.push(id as f32);
        }
    }

    /// Flat quad `x0..x1`, `z1..z0` (`z1 < z0`) at height `y`, both triangles up.
    fn quad(mut self, x0: f32, x1: f32, z0: f32, z1: f32, y: f32, id: i32) -> Self {
        self.triangle(
            Vec3::new(x0, y, z0),
            Vec3::new(x1, y, z0),
            Vec3::new(x0, y, z1),
            id,
        );
        self.triangle(
            Vec3::new(x1, y, z1),
            Vec3::new(x0, y, z1),
            Vec3::new(x1, y, z0),
            id,
        );
        self
    }

    fn build(self) -> WalkMesh {
        WalkMesh::new(self.vertices, self.indices, self.tangents).expect("mesh")
    }
}

fn block_with(meshes: Vec<WalkMesh>, column: usize, row: usize) -> BlockWalk {
    BlockWalk {
        origin: block_origin(column, row),
        form: 1,
        form1: meshes,
        form2: Vec::new(),
    }
}

/// Block (1, 1): land (topograph 5) over local z -30..0, water (topograph 54) over
/// local z -64..-30, both at height 10. World z therefore: land -94..-64.
fn land_and_water() -> WorldWalk {
    let land = MeshBuilder::default()
        .quad(0.0, 64.0, 0.0, -30.0, 10.0, topo_id(5))
        .build();
    let water = MeshBuilder::default()
        .quad(0.0, 64.0, -30.0, -64.0, 10.0, topo_id(54))
        .build();
    let mut world = WorldWalk::new();
    world.set_block(1, 1, block_with(vec![land, water], 1, 1));
    world
}

fn walker_at(world: &WorldWalk, pos: Vec3) -> FootWalker {
    let mut walker = FootWalker::new(0, pos, 0.0).expect("foot control");
    walker.place(world, pos);
    walker
}

/// Stick that moves the actor toward `-z` with camera yaw 0: `vy = -128` after
/// `ly = -vy` gives atan2(128, 0) = 90 degrees, so move direction 0.
fn push_north() -> FrameInputs {
    FrameInputs {
        pad: PadInput {
            stick_x: 0,
            stick_y: -128,
            ..PadInput::default()
        },
        ..FrameInputs::default()
    }
}

#[test]
fn block_and_cell_addressing_follow_the_source() {
    // WMWorld.GetAbsoluteBlock (WMWorld.cs:299): (int)(x / 64), (int)(|z| / 64).
    assert_eq!(block_index(100.0, -130.0), Some((1, 2)));
    assert_eq!(block_index(0.0, 0.0), Some((0, 0)));
    assert_eq!(block_index(63.99, -63.99), Some((0, 0)));
    assert_eq!(block_index(64.0, -64.0), Some((1, 1)));
    assert_eq!(block_index(24.0 * 64.0, 0.0), None);
    assert_eq!(block_index(0.0, -20.0 * 64.0), None);
    assert_eq!(block_index(-70.0, 0.0), None);
    // (int) truncates toward zero, so -63 still reads as column 0.
    assert_eq!(block_index(-63.0, 0.0), Some((0, 0)));
    // ff9.w_worldPos2Cell (ff9.cs:8941): (int)(x / 32) and (int)(z / -32), row * 48.
    assert_eq!(cell_xz(100.0, -130.0), (3, 4));
    assert_eq!(cell_index(100.0, -130.0), 4 * 48 + 3);
    assert_eq!(cell_index(0.5, -0.5), 0);
    // Block origins (WMWorld.OnInitialize, WMWorld.cs:455).
    assert_eq!(block_origin(3, 2), Vec3::new(192.0, 0.0, -128.0));
    // Fixed point is 1/256 unit (ff9.cs:2403-2405).
    assert_eq!(fixed_to_unity(112), 0.4375);
    assert_eq!(unity_to_fixed(0.4375), 112);
    assert_eq!(unity_to_fixed(-0.001), 0);
    // Map id bit fields (ff9.cs:2332-2342).
    let id = (2 << 14) | (7 << 8) | (36 << 2);
    assert_eq!(id_event(id), 2);
    assert_eq!(id_area(id), 7);
    assert_eq!(id_topograph(id), 36);
}

#[test]
fn block_layout_maps_initial_and_current_positions() {
    let mut layout = BlockLayout::identity();
    assert_eq!(
        layout.absolute_block(Vec3::new(100.0, 0.0, -74.0)),
        Some((1, 1))
    );
    // Blocks[0, 0] is the block that started at (3, 2) and now sits at (5, 1).
    layout.set(0, 0, (3, 2), (5, 1));
    assert_eq!(
        layout.absolute_block(Vec3::new(10.0, 0.0, -5.0)),
        Some((5, 1))
    );
    // GetAbsolutePositionOf (WMWorld.cs:253): initial * 64 + (x % 64), -initial * 64 + (z % 64).
    let absolute = layout
        .absolute_position_of(Vec3::new(10.0, 7.0, -5.0))
        .unwrap();
    assert_eq!(
        absolute,
        Vec3::new(3.0 * 64.0 + 10.0, 7.0, -2.0 * 64.0 - 5.0)
    );
    // GetAbsolutePositionOf_FixedPoint (WMWorld.cs:266).
    assert_eq!(
        layout.absolute_position_fixed(100, -50),
        Some((3 * 16384 + 100, -2 * 16384 - 50))
    );
}

#[test]
fn control_table_and_topograph_bits_match_the_source() {
    let foot = &CONTROL_TABLE[0];
    assert_eq!((foot.kind, foot.speed_move, foot.flg_gake), (0, 112, 0));
    assert_eq!(foot.limit, [0x0010_667F, 0xD8FF_3CFF]);
    assert_eq!(CONTROL_TABLE[6].limit, [0x7FFF_667F, 0xD8FF_3CFF]);
    // Walkable on foot: ground, forests (36-38), bridge (52).
    for walkable in [0, 5, 7, 10, 13, 16, 23, 27, 36, 37, 38, 52] {
        assert!(
            check_topograph(&foot.limit, topo_id(walkable)),
            "topograph {walkable}"
        );
    }
    // Blocked on foot: mountain cliff (49), waters (48, 50, 51, 53-57), 8, 9.
    for blocked in [8, 9, 48, 49, 50, 51, 53, 54, 55, 56, 57] {
        assert!(
            !check_topograph(&foot.limit, topo_id(blocked)),
            "topograph {blocked}"
        );
    }
    // Event and area bits do not matter, only bits 2-7.
    assert!(check_topograph(
        &foot.limit,
        (1 << 14) | (3 << 8) | topo_id(5) | 3
    ));
    // 0x23ED0000 marks the water topographs (48, 50, 51, 53-57, 61).
    assert!(check_topograph(&WATER_STATUS, topo_id(54)));
    assert!(!check_topograph(&WATER_STATUS, topo_id(5)));
    assert!(check_topograph(&GROUND_STATUS, topo_id(5)));
    // Light-blue chocobo (control 2) may enter shallow water; foot may not.
    assert!(check_topograph(&CONTROL_TABLE[2].limit, topo_id(54)));
}

#[test]
fn sink_heights_follow_w_movement_get_slice_height() {
    // Forest: row 1 context 0 = 400, minus 100 = 300, S(-300) = -1.171875, forest flag.
    assert_eq!(slice_height(1, topo_id(36)), (-1.171_875, true));
    // Shallow water (54): 250 - 100 = 150 -> -0.5859375.
    assert_eq!(slice_height(1, topo_id(54)), (-0.585_937_5, false));
    // Plain ground: context 8 is 0, so no sink.
    assert_eq!(slice_height(1, topo_id(5)), (0.0, false));
    // slice_type 0 never sinks.
    assert_eq!(slice_height(0, topo_id(54)), (0.0, false));
}

#[test]
fn ray_picks_first_upfacing_triangle_and_skips_exceptions() {
    // Wall (vertical triangle) first in order, then excluded id 4078, then floor.
    let mut builder = MeshBuilder::default();
    // Vertical wall: normal has y = 0, skipped by the up-normal rule (WMPhysics.cs:20).
    builder.triangle(
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(0.0, 20.0, -1.0),
        Vec3::new(64.0, 0.0, -1.0),
        topo_id(5),
    );
    // Excluded id 4078 floor above the real floor (WMPhysics.cs:15).
    builder.triangle(
        Vec3::new(0.0, 15.0, 0.0),
        Vec3::new(64.0, 15.0, 0.0),
        Vec3::new(0.0, 15.0, -64.0),
        4078,
    );
    let wall_and_exception = builder.build();
    let floor = MeshBuilder::default()
        .quad(0.0, 64.0, 0.0, -64.0, 10.0, topo_id(5))
        .build();
    let mut world = WorldWalk::new();
    world.set_block(0, 0, block_with(vec![wall_and_exception, floor], 0, 0));
    let mode = RaycastMode::default();
    let hit = world.nwp_hit(Vec3::new(10.0, 12.0, -10.0), &mode, None);
    assert_eq!((hit.pno, hit.id), (1000, topo_id(5)));
    assert!(near(hit.height, 10.0));
    assert_eq!(hit.walk_mesh, 1);
    // With IgnoreExceptions the excluded floor at 15 is the first hit (origin is
    // 12 + 2.34375 = 14.34, below 15, so the plane is above the origin: no hit).
    let ignoring = RaycastMode {
        ignore_exceptions: true,
        ..RaycastMode::default()
    };
    let hit = world.nwp_hit(Vec3::new(10.0, 12.0, -10.0), &ignoring, None);
    assert!(near(hit.height, 10.0));
    // From the sky (origin +400) the ignored floor at 15 is hit first.
    let sky = RaycastMode {
        cast_from_sky: true,
        ignore_exceptions: true,
        ..RaycastMode::default()
    };
    let hit = world.nwp_hit(Vec3::new(10.0, 12.0, -10.0), &sky, None);
    assert!(near(hit.height, 15.0));
    assert_eq!(hit.id, 4078);
    // Not ignoring exceptions from the sky skips it: the wall (up normal 0) and the
    // excluded floor are skipped, the real floor is hit.
    let sky_strict = RaycastMode {
        cast_from_sky: true,
        ..RaycastMode::default()
    };
    let hit = world.nwp_hit(Vec3::new(10.0, 12.0, -10.0), &sky_strict, None);
    assert!(near(hit.height, 10.0));
    // A block that is not loaded, and a position off the grid, return the miss values.
    let miss = world.nwp_hit(Vec3::new(200.0, 12.0, -10.0), &mode, None);
    assert_eq!((miss.pno, miss.id, miss.height), (-1, 0, 0.0));
    let off = world.nwp_hit(Vec3::new(-100.0, 12.0, -10.0), &mode, None);
    assert_eq!(off.pno, -1);
}

#[test]
fn map_id_31ee_is_accepted_only_for_flying_control() {
    let restricted = MeshBuilder::default()
        .quad(0.0, 64.0, 0.0, -64.0, 20.0, 0x31EE)
        .build();
    let ground = MeshBuilder::default()
        .quad(0.0, 64.0, 0.0, -64.0, 10.0, topo_id(5))
        .build();
    let mut world = WorldWalk::new();
    world.set_block(0, 0, block_with(vec![restricted, ground], 0, 0));
    let probe = Vec3::new(10.0, 18.0, -10.0);
    // WMBlock.Raycast (WMBlock.cs:165): 0x31EE rejected unless control type 1, then the
    // next mesh is tried.
    let foot = world.nwp_hit(probe, &RaycastMode::default(), None);
    assert_eq!((foot.walk_mesh, foot.id), (1, topo_id(5)));
    assert!(near(foot.height, 10.0));
    let flying = RaycastMode {
        control_type: 1,
        ..RaycastMode::default()
    };
    let air = world.nwp_hit(probe, &flying, None);
    assert_eq!((air.walk_mesh, air.id), (0, 0x31EE));
    assert!(near(air.height, 20.0));
}

#[test]
fn move_cache_tries_the_cached_triangle_first() {
    // Two overlapping floors. Mesh order says floor A (10) first.
    let a = MeshBuilder::default()
        .quad(0.0, 64.0, 0.0, -64.0, 10.0, topo_id(5))
        .build();
    let b = MeshBuilder::default()
        .quad(0.0, 64.0, 0.0, -64.0, 11.0, topo_id(6))
        .build();
    let mut world = WorldWalk::new();
    world.set_block(0, 0, block_with(vec![a, b], 0, 0));
    let mode = RaycastMode::default();
    let probe = Vec3::new(10.0, 12.0, -10.0);

    let mut cache = MoveCache::default();
    let first = world.nwp_hit(probe, &mode, Some(&mut cache));
    assert_eq!((first.walk_mesh, first.id), (0, topo_id(5)));
    // A miss path wrote entry 1 (Number++ % 10, WMBlock.cs:130).
    assert_eq!(cache.number, 1);
    assert_eq!(cache.blocks[1], Some((0, 0)));
    assert_eq!(cache.walk_mesh_indices[1], 0);

    // Drop entry 1 (floor A) and point entry 2 at floor B's first triangle: the ring
    // is searched from `number` upward, so B wins over mesh order.
    cache.blocks[1] = None;
    cache.blocks[2] = Some((0, 0));
    cache.walk_mesh_indices[2] = 1;
    cache.triangle_indices[2] = 0;
    let cached = world.nwp_hit(probe, &mode, Some(&mut cache));
    assert_eq!((cached.walk_mesh, cached.id), (1, topo_id(6)));
    assert!(near(cached.height, 11.0));
    // A cache hit does not advance the ring.
    assert_eq!(cache.number, 1);
    // Without a cache the first mesh wins.
    let plain = world.nwp_hit(probe, &mode, None);
    assert_eq!(plain.walk_mesh, 0);
}

#[test]
fn degenerate_triangle_counts_as_a_hit_only_on_the_specified_triangle_path() {
    // WMPhysics.intersect3D_RayTriangle returns -1 for a zero normal; Raycast needs 1,
    // RaycastOnSpecifiedTriangle accepts any non-zero result (WMPhysics.cs:52).
    let flat = [Vec3::new(1.0, 0.0, 1.0); 3];
    assert_eq!(
        intersect_ray_triangle(Vec3::new(0.0, 5.0, 0.0), Vec3::DOWN, flat).0,
        -1
    );
    let mut builder = MeshBuilder::default();
    builder.triangle(flat[0], flat[1], flat[2], topo_id(5));
    let mesh = builder.build();
    assert!(
        raycast_mesh(
            &mesh,
            Vec3::ZERO,
            Vec3::new(0.0, 5.0, 0.0),
            Vec3::DOWN,
            true
        )
        .is_none()
    );
    assert!(raycast_triangle(&mesh, 0, Vec3::ZERO, Vec3::new(0.0, 5.0, 0.0), Vec3::DOWN).is_some());
    // Plane above the ray origin is a miss (t < 0).
    let tri = [
        Vec3::new(0.0, 8.0, 0.0),
        Vec3::new(4.0, 8.0, 0.0),
        Vec3::new(0.0, 8.0, -4.0),
    ];
    assert_eq!(
        intersect_ray_triangle(Vec3::new(1.0, 5.0, -1.0), Vec3::DOWN, tri).0,
        0
    );
    assert_eq!(
        intersect_ray_triangle(Vec3::new(1.0, 9.0, -1.0), Vec3::DOWN, tri).0,
        1
    );
}

#[test]
fn walking_north_on_land_moves_one_step_per_frame_then_slides_and_blocks_at_water() {
    let world = land_and_water();
    let start = Vec3::new(100.0, 10.0, -74.0);
    let mut walker = walker_at(&world, start);
    assert_eq!(walker.status.id, topo_id(5));
    assert!(near(walker.pos.y, 10.0));

    // Foot speed: S(112 * 4096 >> 12) = 112 / 256 = 0.4375 per frame (ff9.cs:5878).
    let mut inputs = push_north();
    let first = walker.step(&world, &inputs);
    assert!(near(first.pos.z, -74.4375));
    assert!(near(first.pos.x, 100.0));
    assert!(near(first.pos.y, 10.0));
    assert!(!first.blocked);
    assert_eq!(first.deflection, Some(0.0));
    assert_eq!(first.block, Some((1, 1)));
    // cell: x 100 / 32 = 3, z -74.4375 / -32 = 2.33 -> 2: 2 * 48 + 3.
    assert_eq!(first.cell, 99);
    assert_eq!(first.topograph, 5);
    assert_eq!(walker.status.id, topo_id(5));

    // 44 more steps reach z = -74 - 45 * 0.4375 = -93.6875, 0.3125 from the water.
    for _ in 0..44 {
        walker.step(&world, &inputs);
    }
    assert!(near(walker.pos.z, -93.6875));

    // Step 46 would end at -94.125 (water). Rotated tries: +-11.25 .. each move at
    // most 0.4375 * cos(a); only +45 degrees stays on land: 0.4375 * 0.70711 = 0.30936
    // <= 0.3125. The positive rotation is tried first.
    let slide = walker.step(&world, &inputs);
    assert_eq!(slide.deflection, Some(45.0));
    assert!(!slide.blocked);
    assert!(near(slide.pos.z, -93.6875 - 0.309_359));
    // heading 180 + 45 = 225 degrees: x moves by sin(225) * 0.4375 = -0.309359.
    assert!(near(slide.pos.x, 100.0 - 0.309_359));

    // Now 0.003 from the edge: even 78.75 degrees (cos = 0.19509) moves 0.0853, so no
    // rotation stays on land and the actor stays put.
    let before = walker.pos;
    let blocked = walker.step(&world, &inputs);
    assert!(blocked.blocked);
    assert_eq!(blocked.deflection, None);
    assert_eq!(walker.pos, before);
    assert_eq!(walker.status.id, topo_id(5));

    // Releasing the stick clears the blocked flag and the speed.
    inputs.pad = PadInput::default();
    let idle = walker.step(&world, &inputs);
    assert!(!idle.blocked);
    assert_eq!(walker.xz_speed, 0.0);
}

#[test]
fn camera_yaw_follows_the_walk_direction() {
    let world = land_and_water();
    let mut walker = walker_at(&world, Vec3::new(100.0, 10.0, -74.0));
    // Stick right: left_x = 128, left_y = 0, direction = -atan2(0, 128) + 90 = 90.
    let inputs = FrameInputs {
        pad: PadInput {
            stick_x: 128,
            ..PadInput::default()
        },
        ..FrameInputs::default()
    };
    let report = walker.step(&world, &inputs);
    // rot_true = (camera 0 + 90) % 360; the actor yaw closes a third of the gap
    // (ff9.cs:5893): 0 + 90 / 3 = 30.
    assert!(near(walker.rot_true, 90.0));
    assert!(near(walker.rot_y, 30.0));
    assert!(near(walker.camera.rotation_max, 1.0));
    assert!(near(walker.camera.rotation_rev, 0.0));
    // heading 90 + 180 = 270: x moves by -0.4375 (screen right is -x).
    assert!(near(report.pos.x, 100.0 - 0.4375));
    // HumanCamOperation (ff9.cs:6158): difference = (90 - 0) * 270 / 360 = 67.5,
    // cn2 = clamp(67.5, 54.6875, 256) = 67.5, factor 1,
    // speed = ((180 - 67.5) * 32 + 2048) * 2 = 11296, remain 4096:
    // rotation += 11296 / 4096 = 2.7578125.
    assert!(near(walker.camera.rotation, 2.757_812_5));
    assert_eq!(walker.camera.remain, 4096.0);
    // With the stick released the remain value falls by 512 per frame
    // (ff9.cs:6171-6173).
    walker.step(&world, &FrameInputs::default());
    assert_eq!(walker.camera.remain, 3584.0);
    // L1 / R1 turn the camera by PsxRot(32) = 2.8125 degrees and block the follow.
    let mut turner = walker_at(&world, Vec3::new(100.0, 10.0, -74.0));
    let manual = FrameInputs {
        pad: PadInput {
            r1: true,
            ..PadInput::default()
        },
        ..FrameInputs::default()
    };
    turner.step(&world, &manual);
    assert!(near(turner.camera.rotation, 2.8125));
    assert!(turner.camera.manual_turn);
}

#[test]
fn forest_sinks_at_once_and_water_sinks_by_steps() {
    let forest = MeshBuilder::default()
        .quad(0.0, 64.0, 0.0, -64.0, 10.0, topo_id(36))
        .build();
    let mut world = WorldWalk::new();
    world.set_block(0, 0, block_with(vec![forest], 0, 0));
    // Forest: slice height = -1.171875 immediately (ff9.cs:5232 `inForest`).
    let walker = walker_at(&world, Vec3::new(10.0, 10.0, -10.0));
    assert_eq!(walker.status.slice_height, -1.171_875);
    assert!(near(walker.pos.y, 10.0 - 1.171_875));
    assert_eq!(walker.status.ground_height, 10.0);

    // Water: -0.078125 per frame toward -0.5859375 (ff9.cs:5236-5239).
    let water = MeshBuilder::default()
        .quad(0.0, 64.0, 0.0, -64.0, 10.0, topo_id(54))
        .build();
    let mut world = WorldWalk::new();
    world.set_block(0, 0, block_with(vec![water], 0, 0));
    let mut walker = walker_at(&world, Vec3::new(10.0, 10.0, -10.0));
    assert_eq!(walker.status.slice_height, -0.078_125);
    walker.step(&world, &FrameInputs::default());
    assert_eq!(walker.status.slice_height, -0.156_25);
    assert!(near(walker.pos.y, 10.0 - 0.156_25));
}

#[test]
fn chocobo_cliff_rule_blocks_ground_to_water_even_when_the_limit_allows_it() {
    let world = land_and_water();
    let mut walker = FootWalker::new(2, Vec3::new(100.0, 10.0, -93.9), 0.0).unwrap();
    walker.place(&world, Vec3::new(100.0, 10.0, -93.9));
    assert!(check_topograph(&CONTROL_TABLE[2].limit, topo_id(54)));
    assert_eq!(CONTROL_TABLE[2].flg_gake, 1);
    // The ground-status actor may not step into water status (ff9.cs:5428-5437).
    let report = walker.step(&world, &push_north());
    assert!(report.blocked);
    assert!(near(walker.pos.z, -93.9));
}

#[test]
fn non_walking_controls_are_rejected() {
    for control in [6, 7, 8, 9, 10] {
        assert_eq!(
            FootWalker::new(control, Vec3::ZERO, 0.0),
            Err(WorldMoveError::UnsupportedControl(control))
        );
    }
    assert_eq!(
        FootWalker::new(12, Vec3::ZERO, 0.0),
        Err(WorldMoveError::UnsupportedControl(12))
    );
}

#[test]
fn event_bits_of_the_ground_id_raise_a_world_event() {
    let id = (1 << 14) | topo_id(5);
    let strip = MeshBuilder::default()
        .quad(0.0, 64.0, 0.0, -64.0, 10.0, id)
        .build();
    let mut world = WorldWalk::new();
    world.set_block(1, 1, block_with(vec![strip], 1, 1));
    let mut walker = walker_at(&world, Vec3::new(100.0, 10.0, -74.0));
    let report = walker.step(&world, &FrameInputs::default());
    // ff9.cs:5086-5090: cell = z * 48 + x with x = 100 / 32 = 3, z = 74 / 32 = 2.
    assert_eq!(
        report.world_event,
        Some(WorldEventHit {
            cell_x: 3,
            cell_z: 2,
            event: 1
        })
    );
    // Disabled event processing raises nothing.
    let quiet = walker.step(
        &world,
        &FrameInputs {
            event_enable: false,
            ..FrameInputs::default()
        },
    );
    assert_eq!(quiet.world_event, None);
}

#[test]
fn encounter_flag_needs_moved_on_the_previous_frame_and_the_mask() {
    let world = land_and_water();
    let mut walker = walker_at(&world, Vec3::new(100.0, 10.0, -74.0));
    let inputs = push_north();
    // Frame 1: Move is still false (set at the end of a frame), so no flag.
    assert!(!walker.step(&world, &inputs).encounter_enabled);
    assert!(walker.moved);
    // Frame 2: the actor moved last frame and the mask is on (ff9.cs:5264).
    assert!(walker.step(&world, &inputs).encounter_enabled);
    let masked = FrameInputs {
        encounter_mask: false,
        ..inputs
    };
    assert!(!walker.step(&world, &masked).encounter_enabled);
}

#[test]
fn encounter_roll_matches_w_frame_get_parameter_205() {
    let base = EncounterCheck {
        no_encounter: false,
        full_map_open: false,
        random_high: 0,
        random_low: 1,
        battle_probability: 9,
        encounter_enabled: true,
        moved: true,
        topograph: 37,
        frame_counter: 401,
    };
    // (0 << 8 | 1) % (9 + 1) == 1: triggers.
    assert!(encounter_triggers(&base));
    // 0x0100 % 10 = 256 % 10 = 6: no.
    assert!(!encounter_triggers(&EncounterCheck {
        random_high: 1,
        random_low: 0,
        ..base
    }));
    // 0x010B = 267, 267 % 10 = 7; choose 0x000B = 11, 11 % 10 = 1: yes.
    assert!(encounter_triggers(&EncounterCheck {
        random_low: 11,
        ..base
    }));
    // Probability 0: x % 1 is always 0, never 1.
    assert!(!encounter_triggers(&EncounterCheck {
        battle_probability: 0,
        ..base
    }));
    // Forest topographs 36..=38 only, and only after frame 400.
    assert!(!encounter_triggers(&EncounterCheck {
        topograph: 35,
        ..base
    }));
    assert!(!encounter_triggers(&EncounterCheck {
        topograph: 39,
        ..base
    }));
    assert!(!encounter_triggers(&EncounterCheck {
        frame_counter: 400,
        ..base
    }));
    assert!(!encounter_triggers(&EncounterCheck {
        moved: false,
        ..base
    }));
    assert!(!encounter_triggers(&EncounterCheck {
        encounter_enabled: false,
        ..base
    }));
    assert!(!encounter_triggers(&EncounterCheck {
        no_encounter: true,
        ..base
    }));
    assert!(!encounter_triggers(&EncounterCheck {
        full_map_open: true,
        ..base
    }));
}

#[test]
fn pad_axes_map_to_sticks_as_in_the_source() {
    // Digital: past 0.1 is +-128; y positive (down) is -128, y negative (up) is 128.
    assert_eq!(stick_from_axis(0.5, 0.0, false), (128, 0));
    assert_eq!(stick_from_axis(-0.5, 0.0, false), (-128, 0));
    assert_eq!(stick_from_axis(0.0, 0.5, false), (0, -128));
    assert_eq!(stick_from_axis(0.0, -0.5, false), (0, 128));
    assert_eq!(stick_from_axis(0.05, -0.05, false), (0, 0));
    // Analog: (int)(x * 128) and (int)(-y * 128).
    assert_eq!(stick_from_axis(0.5, 0.25, true), (64, -32));
    assert_eq!(stick_from_axis(0.0, 0.0, true), (0, 0));
}

#[test]
fn walk_mesh_construction_rejects_malformed_input() {
    let v = vec![Vec3::ZERO; 3];
    assert!(matches!(
        WalkMesh::new(v.clone(), vec![0, 1, 3], vec![0.0; 3]),
        Err(WalkMeshError::IndexOutOfRange { .. })
    ));
    assert!(matches!(
        WalkMesh::new(v.clone(), vec![0, 1, 2], vec![0.0; 2]),
        Err(WalkMeshError::TangentCount { .. })
    ));
    // Two triangles over three vertices: AddWalkMesh builds vertices / 3 = 1 normal.
    assert!(matches!(
        WalkMesh::new(v, vec![0, 1, 2, 0, 1, 2], vec![0.0; 3]),
        Err(WalkMeshError::MoreTrianglesThanNormals { .. })
    ));
}
