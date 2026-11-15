//! `tantalusctl assets inspect-bundle --world-walk <out json>`: loads the disc 1 world
//! blocks from `p0data3.bin`, walks a scripted straight-line path on land with the
//! W8 foot-movement core and records per-step block, cell and triangle.

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::unity_raw::UnityRawBundle;
use tantalus_assets::pc::world_map::unity::{
    BlockCellReport, WalkMeshData, analyze_world_map, extract_walk_meshes,
};
use tantalus_legacy::world::{
    BlockWalk, CONTROL_TABLE, FootWalker, FrameInputs, PadInput, Vec3, WalkMesh, WorldWalk,
    block_origin, check_topograph,
};

const DISC: u32 = 1;
const PATH_STEPS: usize = 200;
const SHORE_PROBE_STEPS: usize = 2000;
/// `WMWorld.LoadBlock` registers these slots in this order for form 1 and form 2
/// (`WM/WMWorld/WMWorld.cs:508-690`).
const FORM1_FIRST: [&str; 4] = [
    "ObjectForm1",
    "TerrainForm1",
    "VolcanoCrater1",
    "VolcanoLava1",
];
const FORM2_FIRST: [&str; 4] = [
    "ObjectForm2",
    "TerrainForm2",
    "VolcanoCrater2",
    "VolcanoLava2",
];
const SHARED: [&str; 12] = [
    "Beach1",
    "Beach2",
    "Stream",
    "River",
    "RiverJoint",
    "Falls",
    "Sea1",
    "Sea2",
    "Sea3",
    "Sea4",
    "Sea5",
    "Sea6",
];
/// Block 219 returns early after object and terrain with only these seas
/// (`WMWorld.cs:518-546`).
const BLOCK_219_FORM1: [&str; 3] = ["Sea3", "Sea4", "Sea5"];
const BLOCK_219_FORM2: [&str; 3] = ["Sea3_2", "Sea4_2", "Sea5_2"];

struct Loaded {
    world: WorldWalk,
    /// Form 1 slot names per block, indexing the walk-mesh numbers in reports.
    slots: HashMap<(usize, usize), Vec<String>>,
    terrain_blocks: Vec<(usize, usize)>,
    failures: Vec<String>,
    meshes: usize,
}

fn slot_list(block_219: bool, form: u8) -> Vec<&'static str> {
    let (first, special): (&[&str], &[&str]) = if form == 1 {
        (&FORM1_FIRST, &BLOCK_219_FORM1)
    } else {
        (&FORM2_FIRST, &BLOCK_219_FORM2)
    };
    let mut list: Vec<&'static str> = first[..2].to_vec();
    if block_219 {
        list.extend_from_slice(special);
    } else {
        list.extend_from_slice(&first[2..]);
        list.extend_from_slice(&SHARED);
    }
    list
}

fn to_walk_mesh(data: &WalkMeshData) -> Result<WalkMesh, String> {
    let vertices = data
        .positions
        .iter()
        .map(|p| Vec3::new(p[0], p[1], p[2]))
        .collect();
    WalkMesh::new(vertices, data.indices.clone(), data.tangent_x.clone())
        .map_err(|error| format!("{}: {error}", data.path))
}

fn load_world(cells: &[&BlockCellReport], meshes: &HashMap<String, WalkMeshData>) -> Loaded {
    let mut loaded = Loaded {
        world: WorldWalk::new(),
        slots: HashMap::new(),
        terrain_blocks: Vec::new(),
        failures: Vec::new(),
        meshes: 0,
    };
    let mut built: HashMap<String, Option<WalkMesh>> = HashMap::new();
    for cell in cells {
        let Some(prefab) = &cell.prefab else { continue };
        let block_219 = prefab.fields.get("Number") == Some(&219);
        let key = (cell.x as usize, cell.y as usize);
        let mut forms: [Vec<WalkMesh>; 2] = [Vec::new(), Vec::new()];
        let mut names = Vec::new();
        for form in [1_u8, 2] {
            for slot_name in slot_list(block_219, form) {
                let Some(slot) = prefab.slots.iter().find(|s| s.slot == slot_name) else {
                    continue;
                };
                let Some(path) = &slot.mesh_path else {
                    continue;
                };
                let entry = built.entry(path.clone()).or_insert_with(|| {
                    match meshes.get(path).map(to_walk_mesh) {
                        Some(Ok(mesh)) => Some(mesh),
                        Some(Err(message)) => {
                            loaded.failures.push(message);
                            None
                        }
                        None => {
                            loaded.failures.push(format!("{path}: mesh not extracted"));
                            None
                        }
                    }
                });
                if let Some(mesh) = entry {
                    forms[usize::from(form) - 1].push(mesh.clone());
                    if form == 1 {
                        names.push(slot_name.to_owned());
                    }
                    loaded.meshes += 1;
                }
            }
        }
        if prefab.slots.iter().any(|s| s.slot == "TerrainForm1") {
            loaded.terrain_blocks.push(key);
        }
        let [form1, form2] = forms;
        loaded.world.set_block(
            key.0,
            key.1,
            BlockWalk {
                origin: block_origin(key.0, key.1),
                form: 1,
                form1,
                form2,
            },
        );
        loaded.slots.insert(key, names);
    }
    loaded.terrain_blocks.sort_unstable();
    loaded
}

struct Heading {
    name: &'static str,
    stick: (i32, i32),
}

/// Camera yaw 0 looks toward `-z`; stick up is north (`-z`), right is `-x`
/// (`ff9.w_movementHumanOperation`). Only north and south keep the camera still.
static HEADINGS: [Heading; 2] = [
    Heading {
        name: "north (-z)",
        stick: (0, -128),
    },
    Heading {
        name: "south (+z)",
        stick: (0, 128),
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum StopOn {
    Never,
    Blocked,
    /// A blocked or deflected step ends the walk (used while searching).
    Any,
}

struct Walk {
    /// Steps before the first stop condition.
    clean_steps: usize,
    steps: Vec<Value>,
    blocked: usize,
    /// Steps that only succeeded after the 11.25-degree rotation search.
    deflected: usize,
    blocks: BTreeSet<(usize, usize)>,
    cells: BTreeSet<i32>,
    topographs: BTreeSet<i32>,
    crossings: usize,
    first_blocked: Option<usize>,
}

fn run_walk(
    loaded: &Loaded,
    start: Vec3,
    heading: &Heading,
    steps: usize,
    stop_on: StopOn,
    record: bool,
) -> Option<Walk> {
    let mut walker = FootWalker::new(0, start, 0.0).ok()?;
    walker.place(&loaded.world, start);
    if walker.status.ground_height == 0.0 && walker.status.id == 0 {
        return None;
    }
    if !check_topograph(&CONTROL_TABLE[0].limit, walker.status.id) {
        return None;
    }
    let inputs = FrameInputs {
        pad: PadInput {
            stick_x: heading.stick.0,
            stick_y: heading.stick.1,
            ..PadInput::default()
        },
        ..FrameInputs::default()
    };
    let mut walk = Walk {
        clean_steps: 0,
        steps: Vec::new(),
        blocked: 0,
        deflected: 0,
        blocks: BTreeSet::new(),
        cells: BTreeSet::new(),
        topographs: BTreeSet::new(),
        crossings: 0,
        first_blocked: None,
    };
    let mut previous_block = None;
    for step in 1..=steps {
        let report = walker.step(&loaded.world, &inputs);
        if report.blocked {
            walk.blocked += 1;
            walk.first_blocked.get_or_insert(step);
        }
        if report.deflection.is_some_and(|d| d != 0.0) {
            walk.deflected += 1;
        }
        if let Some(block) = report.block {
            walk.blocks.insert(block);
            if previous_block.is_some_and(|p| p != block) {
                walk.crossings += 1;
            }
            previous_block = Some(block);
        }
        walk.cells.insert(report.cell);
        walk.topographs.insert(report.topograph);
        if record {
            let slot = report
                .block
                .and_then(|b| loaded.slots.get(&b))
                .and_then(|names| {
                    usize::try_from(report.walk_mesh)
                        .ok()
                        .and_then(|i| names.get(i))
                })
                .cloned();
            walk.steps.push(json!({
                "step": step,
                "position": [report.pos.x, report.pos.y, report.pos.z],
                "block": report.block.map(|(x, z)| [x, z]),
                "cell": report.cell,
                "cell_xz": [report.cell % 48, report.cell / 48],
                "topograph": report.topograph,
                "map_id": report.id,
                "walk_mesh_slot": slot,
                "triangle": report.triangle,
                "blocked": report.blocked,
                "deflection_degrees": report.deflection,
            }));
        }
        let deflected = report.deflection.is_some_and(|d| d != 0.0);
        match stop_on {
            StopOn::Never => {}
            StopOn::Blocked if report.blocked => break,
            StopOn::Any if report.blocked || deflected => break,
            _ => {}
        }
        walk.clean_steps = step;
    }
    Some(walk)
}

fn find_path(loaded: &Loaded) -> Option<(Vec3, &'static Heading, usize, bool)> {
    let mut best: Option<(Vec3, &'static Heading, usize, usize)> = None;
    for &(bx, bz) in &loaded.terrain_blocks {
        let origin = block_origin(bx, bz);
        for i in 1..8 {
            for j in 1..8 {
                let start = Vec3::new(origin.x + i as f32 * 8.0, 0.0, origin.z - j as f32 * 8.0);
                for heading in &HEADINGS {
                    let Some(walk) =
                        run_walk(loaded, start, heading, PATH_STEPS, StopOn::Any, false)
                    else {
                        continue;
                    };
                    if walk.clean_steps == PATH_STEPS && walk.blocks.len() >= 2 {
                        return Some((start, heading, PATH_STEPS, true));
                    }
                    let score = (walk.clean_steps, walk.blocks.len());
                    if best.is_none_or(|b| score > (b.2, b.3)) {
                        best = Some((start, heading, walk.clean_steps, walk.blocks.len()));
                    }
                }
            }
        }
    }
    let (start, heading, clean, _) = best?;
    Some((start, heading, clean.max(1), false))
}

pub fn run_world_walk(path: &Path, out: &Path, json_output: bool) -> Result<(), String> {
    let source = path.to_string_lossy().into_owned();
    let bundle = UnityRawBundle::open(path).map_err(|error| error.to_string())?;
    let bytes = bundle
        .read_entry(0, 600 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    let serialized = SerializedFile::parse(&bytes, &source).map_err(|error| error.to_string())?;
    let report = analyze_world_map(&serialized, &source).map_err(|error| error.to_string())?;
    let cells: Vec<&BlockCellReport> = report.blocks.iter().filter(|b| b.disc == DISC).collect();
    let mut wanted = BTreeSet::new();
    for cell in &cells {
        if let Some(prefab) = &cell.prefab {
            for slot in &prefab.slots {
                if let Some(mesh) = &slot.mesh_path {
                    wanted.insert(mesh.clone());
                }
            }
        }
    }
    let (extracted, skipped) =
        extract_walk_meshes(&serialized, &source, &|p: &str| wanted.contains(p))
            .map_err(|error| error.to_string())?;
    let meshes: HashMap<String, WalkMeshData> =
        extracted.into_iter().map(|m| (m.path.clone(), m)).collect();
    let mut loaded = load_world(&cells, &meshes);
    loaded.failures.extend(skipped);
    let (start, heading, steps, clean) =
        find_path(&loaded).ok_or_else(|| "no land start point found".to_owned())?;
    let walk = run_walk(&loaded, start, heading, steps, StopOn::Never, true)
        .ok_or_else(|| "path replay failed".to_owned())?;
    // Keep walking the same heading to show where collision first stops the actor.
    let shore = run_walk(
        &loaded,
        start,
        heading,
        steps + SHORE_PROBE_STEPS,
        StopOn::Blocked,
        true,
    )
    .map(|probe| {
        let last = probe.steps.last().cloned();
        json!({
            "steps_taken": probe.steps.len(),
            "first_blocked_step": probe.first_blocked,
            "last_step": last,
        })
    });
    let mut by_topograph: BTreeMap<String, usize> = BTreeMap::new();
    for step in &walk.steps {
        *by_topograph
            .entry(step["topograph"].to_string())
            .or_insert(0) += 1;
    }
    let audit = json!({
        "source": "Memoria c716e1fa (WM/WMBlock, WM/WMPhysics, ff9/ff9.cs w_movement*)",
        "disc": DISC,
        "blocks_loaded": cells.iter().filter(|c| c.prefab.is_some()).count(),
        "walk_meshes_registered": loaded.meshes,
        "mesh_failures": loaded.failures,
        "path_is_straight_land_crossing": clean,
        "heading": heading.name,
        "stick": [heading.stick.0, heading.stick.1],
        "start": [start.x, start.y, start.z],
        "summary": {
            "steps": walk.steps.len(),
            "blocked_steps": walk.blocked,
            "deflected_steps": walk.deflected,
            "distinct_blocks": walk.blocks.len(),
            "block_crossings": walk.crossings,
            "distinct_cells": walk.cells.len(),
            "topographs": walk.topographs,
            "steps_per_topograph": by_topograph,
        },
        "continued_until_blocked": shore,
        "steps": walk.steps,
    });
    let text = serde_json::to_string_pretty(&audit).map_err(|error| error.to_string())?;
    std::fs::write(out, format!("{text}\n"))
        .map_err(|error| format!("cannot write {}: {error}", out.display()))?;
    if json_output {
        println!("{text}");
    } else {
        println!(
            "world walk: {} {} from ({:.3}, {:.3}, {:.3}): {} steps, {} blocked, {} deflected, {} blocks, {} crossings, {} cells; wrote {}",
            if clean { "straight" } else { "best-effort" },
            heading.name,
            start.x,
            start.y,
            start.z,
            walk.steps.len(),
            walk.blocked,
            walk.deflected,
            walk.blocks.len(),
            walk.crossings,
            walk.cells.len(),
            out.display()
        );
    }
    Ok(())
}
