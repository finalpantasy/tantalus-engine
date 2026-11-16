//! Read-only tracing tools for real event archives.
//!
//! `tantalus-trace disasm --bundle <p0data7.bin> --event <NAME>.eb [--entry N]`
//! prints every decoded instruction of an archive.
//!
//! `tantalus-trace chain --bundle <p0data7.bin> --event-db <FF9DBAll.Events.cs>
//! --atan-table <ratan_tbl.bin> [--start 70] [--max-fields 12] [--ticks 4000]`
//! starts at a field map number, runs its event archive under the
//! orchestrated `FieldDirector` (tag 0 per entry, requested tags spawned on
//! demand, as in `tantalusctl --orchestrate`), and follows each `MAPJUMP`
//! field change to the next archive, printing one JSON record per field. The
//! map-number-to-archive table is read at run time from the pinned Memoria
//! source file; it is not copied into this repository.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tantalus_contracts::EntityId;
use tantalus_legacy::{
    EventBinary, EventProgram, Expression, ExpressionOp, InstructionKind, VmStatus,
};
use tantalus_tools::field_data::{
    FieldText, load_navigation, parse_background_map, parse_event_db,
};
use tantalus_tools::field_session::{
    EntryScheduler, decode_pool, function_entity, load_event, register,
};

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("tantalus-trace: {message}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "usage: tantalus-trace disasm --bundle <path> --event <name>.eb [--entry N]
       tantalus-trace fight --bundle <path> --battles <battle-scene-ids.tsv> --character-data <dir> --battle N [--party 0,9,10] [--globals SCENARIO:TRANCE:DEPRESS] [--items ID:COUNT,...] [--hp CHARACTER:HP,...]
       tantalus-trace chain --bundle <path> --event-db <FF9DBAll.Events.cs> --atan-table <path> [--start N] [--max-fields N] [--ticks N] [--confirm-after N] [--explore] [--walk-demo] [--character-data <Data/Characters dir>] [--item-table <Assembly-CSharp.dll>] [--game-data <FF9_Data dir> --mes-map <EventEngineUtils.cs> [--language us]] [--choices MAP:TEXT=N[@SCENARIO],...] [--route MAP>DEST[@SCENARIO],...] [--skip-talk MAP:ENTRY,...] [--hold MAP:MASK,...] [--waypoints MAP@X:Z/X:Z,...] [--battles <battle-scene-ids.tsv>] [--world-exits WORLD:LOCATION[@SCENARIO],...] [--actor-track <dir>]";

fn run(args: Vec<String>) -> Result<(), String> {
    let mut iter = args.into_iter();
    let command = iter.next().ok_or(USAGE)?;
    let mut bundle = None;
    let mut event = None;
    let mut only_entry = None;
    let mut event_db = None;
    let mut atan_table = None;
    let mut start = 70i32;
    let mut max_fields = 12usize;
    let mut ticks = 4000u64;
    let mut confirm_after = 0u64;
    let mut explore = false;
    let mut walk_demo = false;
    let mut actor_track: Option<PathBuf> = None;
    let mut world_exits: Vec<(i32, i32, i32)> = Vec::new();
    let mut character_data: Option<PathBuf> = None;
    let mut game_data: Option<PathBuf> = None;
    let mut mes_map: Option<PathBuf> = None;
    let mut item_table: Option<PathBuf> = None;
    let mut language = String::from("us");
    // (map, text, first scenario) -> answer.
    let mut choices: BTreeMap<(i32, u16, i32), i32> = BTreeMap::new();
    let mut route: Vec<(i32, i32, i32)> = Vec::new();
    let mut skip_talk: BTreeSet<(i32, usize)> = BTreeSet::new();
    let mut hold: BTreeMap<i32, i32> = BTreeMap::new();
    let mut fight_battle: Option<i32> = None;
    let mut fight_party: Vec<i32> = vec![0];
    let mut fight_globals = tantalus_tools::field_battle::PartyGlobals::default();
    let mut fight_items: std::collections::BTreeMap<i32, i32> = Default::default();
    let mut fight_hp: std::collections::BTreeMap<i32, u32> = Default::default();
    let mut battle_ids: Option<PathBuf> = None;
    let mut waypoints: BTreeMap<i32, Vec<[f32; 2]>> = BTreeMap::new();
    while let Some(flag) = iter.next() {
        let mut value = || {
            iter.next()
                .ok_or_else(|| format!("`{flag}` requires a value"))
        };
        match flag.as_str() {
            "--bundle" => bundle = Some(PathBuf::from(value()?)),
            "--event" => event = Some(value()?),
            "--entry" => only_entry = Some(parse_number(&value()?, "--entry")?),
            "--event-db" => event_db = Some(PathBuf::from(value()?)),
            "--atan-table" => atan_table = Some(PathBuf::from(value()?)),
            "--start" => start = parse_number(&value()?, "--start")?,
            "--max-fields" => max_fields = parse_number(&value()?, "--max-fields")?,
            "--ticks" => ticks = parse_number(&value()?, "--ticks")?,
            "--character-data" => character_data = Some(PathBuf::from(value()?)),
            "--item-table" => item_table = Some(PathBuf::from(value()?)),
            "--explore" => explore = true,
            "--walk-demo" => walk_demo = true,
            "--actor-track" => actor_track = Some(PathBuf::from(value()?)),
            "--world-exits" => {
                for part in value()?.split(',') {
                    let (step, from) = match part.trim().split_once('@') {
                        Some((step, from)) => (step, from.parse().ok()),
                        None => (part.trim(), Some(i32::MIN)),
                    };
                    let parsed = step.split_once(':').and_then(|(map, location)| {
                        Some((map.parse().ok()?, location.parse().ok()?, from?))
                    });
                    world_exits.push(parsed.ok_or_else(|| {
                        format!("bad --world-exits entry `{part}`, want WORLD:LOCATION[@SCENARIO]")
                    })?);
                }
            }
            "--game-data" => game_data = Some(PathBuf::from(value()?)),
            "--mes-map" => mes_map = Some(PathBuf::from(value()?)),
            "--language" => language = value()?,
            "--choices" => {
                for part in value()?.split(',') {
                    let parsed = part.trim().split_once(':').and_then(|(map, rest)| {
                        let (text, answer) = rest.split_once('=')?;
                        let (answer, from) = match answer.split_once('@') {
                            Some((answer, from)) => (answer, from.parse().ok()?),
                            None => (answer, 0),
                        };
                        Some((
                            map.parse().ok()?,
                            text.parse().ok()?,
                            from,
                            answer.parse().ok()?,
                        ))
                    });
                    let (map, text, from, answer) = parsed.ok_or_else(|| {
                        format!("bad --choices entry `{part}`, want MAP:TEXT=N[@SCENARIO]")
                    })?;
                    choices.insert((map, text, from), answer);
                }
            }
            "--route" => {
                for part in value()?.split(',') {
                    let (step, from) = match part.trim().split_once('@') {
                        Some((step, from)) => (step, from.parse().ok()),
                        None => (part.trim(), Some(i32::MIN)),
                    };
                    let parsed = step.split_once('>').and_then(|(map, dest)| {
                        Some((map.parse().ok()?, dest.parse().ok()?, from?))
                    });
                    route.push(parsed.ok_or_else(|| {
                        format!("bad --route entry `{part}`, want MAP>DEST[@SCENARIO]")
                    })?);
                }
            }
            "--skip-talk" => {
                for part in value()?.split(',') {
                    let parsed = part
                        .trim()
                        .split_once(':')
                        .and_then(|(map, entry)| Some((map.parse().ok()?, entry.parse().ok()?)));
                    skip_talk.insert(parsed.ok_or_else(|| {
                        format!("bad --skip-talk entry `{part}`, want MAP:ENTRY")
                    })?);
                }
            }
            "--battles" => battle_ids = Some(PathBuf::from(value()?)),
            "--hold" => {
                for part in value()?.split(',') {
                    let parsed = part
                        .trim()
                        .split_once(':')
                        .and_then(|(map, mask)| Some((map.parse().ok()?, mask.parse().ok()?)));
                    let (map, mask) = parsed
                        .ok_or_else(|| format!("bad --hold entry `{part}`, want MAP:MASK"))?;
                    hold.insert(map, mask);
                }
            }
            "--waypoints" => {
                for part in value()?.split(',') {
                    let bad = || format!("bad --waypoints entry `{part}`, want MAP@X:Z/X:Z");
                    let (map, points) = part.trim().split_once('@').ok_or_else(bad)?;
                    let map: i32 = map.parse().map_err(|_| bad())?;
                    for point in points.split('/') {
                        let (x, z) = point.split_once(':').ok_or_else(bad)?;
                        let point = [x.parse().map_err(|_| bad())?, z.parse().map_err(|_| bad())?];
                        waypoints.entry(map).or_default().push(point);
                    }
                }
            }
            "--confirm-after" => confirm_after = parse_number(&value()?, "--confirm-after")?,
            "--battle" => fight_battle = Some(parse_number(&value()?, "--battle")?),
            "--party" => {
                fight_party = value()?
                    .split(',')
                    .map(|id| parse_number(id.trim(), "--party"))
                    .collect::<Result<_, _>>()?;
            }
            "--hp" => {
                let value = value()?;
                for pair in value.split(',') {
                    let (character, hp) =
                        pair.split_once(':').ok_or("--hp wants CHARACTER:HP,...")?;
                    fight_hp.insert(
                        parse_number(character.trim(), "--hp")?,
                        parse_number(hp.trim(), "--hp")?,
                    );
                }
            }
            "--items" => {
                let value = value()?;
                for pair in value.split(',') {
                    let (item, count) = pair.split_once(':').ok_or("--items wants ID:COUNT,...")?;
                    fight_items.insert(
                        parse_number(item.trim(), "--items")?,
                        parse_number(count.trim(), "--items")?,
                    );
                }
            }
            "--globals" => {
                let text = value()?;
                let parts: Vec<i32> = text
                    .split(':')
                    .map(|part| parse_number(part.trim(), "--globals"))
                    .collect::<Result<_, _>>()?;
                let [scenario, trance, depress] = parts[..] else {
                    return Err("--globals wants SCENARIO:TRANCE:DEPRESS".into());
                };
                fight_globals = tantalus_tools::field_battle::PartyGlobals {
                    scenario,
                    trance_gauge: trance as u8,
                    garnet_depress: depress as u8,
                };
            }
            other => return Err(format!("unrecognized argument `{other}`\n{USAGE}")),
        }
    }
    match command.as_str() {
        "disasm" => {
            let bundle = bundle.ok_or("--bundle is required")?;
            let event = event.ok_or("--event is required")?;
            let binary = load_event(&bundle, &event)?;
            disassemble(&binary, only_entry)
        }
        "fight" => {
            // One battle with the new-game party stats, for iterating on
            // the battle machine without the whole chain.
            let bundle = bundle.ok_or("--bundle is required")?;
            let ids = battle_ids.ok_or("--battles is required")?;
            let dir = character_data.ok_or("--character-data is required")?;
            let battle = fight_battle.ok_or("--battle is required")?;
            let read = |name: &str| {
                let path = dir.join(name);
                std::fs::read_to_string(&path)
                    .map_err(|error| format!("cannot read {}: {error}", path.display()))
            };
            let base = tantalus_game::new_game::parse_base_stats(&read("BaseStats.csv")?)?;
            let levels = tantalus_game::new_game::parse_leveling(&read("Leveling.csv")?)?;
            let stats = tantalus_game::new_game::new_game_character_stats(&base, &levels);
            let battles = tantalus_tools::field_battle::FieldBattles::load(
                bundle.parent().ok_or("--bundle has no directory")?,
                &bundle,
                &ids,
                tantalus_tools::field_battle::Equipment::load(&dir)?,
            )?;
            let party: Vec<tantalus_tools::field_battle::PartyFighter> = fight_party
                .iter()
                .filter_map(|character| {
                    let stats = stats.get(character)?;
                    Some(tantalus_tools::field_battle::PartyFighter {
                        character: *character,
                        hp: fight_hp
                            .get(character)
                            .copied()
                            .unwrap_or(stats.current_hp.max(0) as u32),
                        max_hp: stats.max_hp.max(0) as u32,
                        mp: stats.current_mp.max(0) as u32,
                        max_mp: stats.max_mp.max(0) as u32,
                        base: *base.get(character)?,
                        globals: fight_globals,
                        row: 1,
                    })
                })
                .collect();
            let request = tantalus_game::field::world::EncounterRequest {
                rush: 0,
                group: None,
                battle,
                steiner_state: false,
            };
            let report = battles.fight_with_items(&request, &party, &fight_items);
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        "chain" => {
            let bundle = bundle.ok_or("--bundle is required")?;
            let db_path = event_db.ok_or("--event-db is required")?;
            let source = std::fs::read_to_string(&db_path)
                .map_err(|error| format!("cannot read {}: {error}", db_path.display()))?;
            let db = parse_event_db(&source);
            let atan = match atan_table {
                Some(path) => Some(
                    std::fs::read(&path)
                        .map_err(|error| format!("cannot read {}: {error}", path.display()))?,
                ),
                None => None,
            };
            let mut initial_items = BTreeMap::new();
            let mut growth_tables: Option<(
                BTreeMap<i32, tantalus_game::new_game::BaseStats>,
                Vec<tantalus_game::new_game::LevelRow>,
            )> = None;
            let stats = match &character_data {
                Some(dir) => {
                    let read = |name: &str| {
                        let path = dir.join(name);
                        std::fs::read_to_string(&path)
                            .map_err(|error| format!("cannot read {}: {error}", path.display()))
                    };
                    let base = tantalus_game::new_game::parse_base_stats(&read("BaseStats.csv")?)?;
                    let levels = tantalus_game::new_game::parse_leveling(&read("Leveling.csv")?)?;
                    growth_tables = Some((base.clone(), levels.clone()));
                    initial_items = tantalus_game::new_game::parse_initial_items(&read(
                        "../Items/InitialItems.csv",
                    )?)?;
                    tantalus_game::new_game::new_game_character_stats(&base, &levels)
                }
                None => BTreeMap::new(),
            };
            // Reward-item identities: the vanilla item table maps
            // `ffix:item/<legacy id>` onto the ids field scripts use.
            let item_ids: Option<Vec<i32>> = match item_table {
                Some(path) => {
                    use tantalus_assets::pc::battle_tables::extract_battle_tables;
                    use tantalus_assets::pc::managed_il::Assembly;
                    let bytes = std::fs::read(&path)
                        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
                    let assembly = Assembly::parse(&bytes).map_err(|error| error.to_string())?;
                    let tables =
                        extract_battle_tables(&assembly).map_err(|error| error.to_string())?;
                    Some(tables.items.iter().map(|row| row.id as i32).collect())
                }
                None => None,
            };
            let mut backgrounds = BTreeMap::new();
            let text = match (game_data.clone(), mes_map) {
                (Some(data), Some(map_path)) => {
                    // Clone for ChainOptions (the FMV movie inventory).
                    let source = std::fs::read_to_string(&map_path)
                        .map_err(|error| format!("cannot read {}: {error}", map_path.display()))?;
                    backgrounds = parse_background_map(&source);
                    let text = FieldText::open(&data, &source, language)?;
                    // The movies directory is derived from the same root.
                    game_data = Some(data);
                    Some(text)
                }
                (None, None) => None,
                _ => return Err("--game-data and --mes-map go together".into()),
            };
            chain(
                &bundle,
                &db,
                atan,
                ChainOptions {
                    start,
                    max_fields,
                    ticks,
                    confirm_after,
                    explore,
                    walk_demo,
                    actor_track,
                    world_exits,
                    stats,
                    text,
                    choices,
                    game_data,
                    growth_tables,
                    initial_items,
                    item_ids,
                    route,
                    skip_talk,
                    hold,
                    backgrounds,
                    waypoints,
                    battles: match battle_ids {
                        Some(ids) => Some(tantalus_tools::field_battle::FieldBattles::load(
                            bundle.parent().ok_or("--bundle has no directory")?,
                            &bundle,
                            &ids,
                            match &character_data {
                                Some(dir) => tantalus_tools::field_battle::Equipment::load(dir)?,
                                None => Default::default(),
                            },
                        )?),
                        None => None,
                    },
                },
            )
        }
        other => Err(format!("unknown command `{other}`\n{USAGE}")),
    }
}

fn parse_number<T: std::str::FromStr>(text: &str, flag: &str) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    text.parse::<T>()
        .map_err(|error| format!("bad {flag} `{text}`: {error}"))
}

/// Finds a TextAsset by exact name in a UnityRaw bundle and parses it as an
/// event archive.
fn disassemble(binary: &EventBinary, only_entry: Option<usize>) -> Result<(), String> {
    for entry in &binary.entries {
        if only_entry.is_some_and(|wanted| wanted != entry.index) {
            continue;
        }
        let report = binary
            .decode_entry_with_diagnostics(entry.index)
            .map_err(|error| error.to_string())?;
        println!(
            "== entry {} type {} ({} functions, {} errors)",
            entry.index,
            report.entry_type,
            report.functions.len(),
            report.errors.len()
        );
        for function in &report.functions {
            println!(
                "-- function tag {} offset {}",
                function.tag, function.offset
            );
            for instruction in function.program.instructions.values() {
                println!("{:5}: {:?}", instruction.offset, instruction.kind);
            }
        }
        for error in &report.errors {
            println!(
                "!! function {} tag {}: {}",
                error.function, error.tag, error.error
            );
        }
    }
    Ok(())
}

/// Nonzero bytes as `{offset: value}` for compact memory dumps.
fn nonzero(bytes: &[u8]) -> BTreeMap<usize, u8> {
    bytes
        .iter()
        .enumerate()
        .filter(|(_, byte)| **byte != 0)
        .map(|(offset, byte)| (offset, *byte))
        .collect()
}

/// `EventInput.Confirm`.
const CONFIRM: i32 = 0x20000;
/// `EventInput.Cancel | EventInput.Special`. Western builds test script
/// key masks raw (`ETb.KeyOn` swaps Confirm and Cancel only for the
/// Japanese layout), so a few scenes wait on these (map 65 entry 3,
/// `KeyOn(0x10000)` or `KeyOn(0x80000)` after text 241).
const CANCEL_SPECIAL: i32 = 0x90000;
/// Ticks without control or new text before the cutscene taps add
/// Cancel and Special to Confirm.
const STALL_TICKS: u64 = 300;
const CONFIRM_PERIOD: u64 = 15;
/// Refusals (a line shown, the player kept) before the explorer tries
/// another exit.
const EXIT_REFUSALS: u32 = 3;
/// Button prompts the explorer answers: (map, text id, `KeyOn` mask). Map
/// 64's sword fight (`EVT_ALEX1_TS_SWD_BTL` entry 3) shows "Press [n]!"
/// as text 112 + n for byte 46 = n and checks one key per n: Left, Right,
/// Triangle, Down, Cross, Up, Circle, Square. Map 206's ATE tutorial
/// (`EVT_DOWNSHIP_BT_FRT_0` entry 0) holds text 352 until Select (1) is held.
const PROMPT_KEYS: [(i32, u16, i32); 9] = [
    (206, 352, 0x1),
    (64, 112, 0x80),
    (64, 113, 0x20),
    (64, 114, 0x1000),
    (64, 115, 0x40),
    (64, 116, 0x4000),
    (64, 117, 0x10),
    (64, 118, 0x2000),
    (64, 119, 0x8000),
];
/// Ticks the scripted player stays in one region before moving on.
const REGION_DWELL: u64 = 120;
/// Ticks the explorer waits next to an object for its talk to start.
const TALK_DWELL: u64 = 45;
/// Ticks the scripted player stands on each `--waypoints` position.
const WAYPOINT_DWELL: u64 = 60;
/// X offset from an object where the explorer stands to talk (inside the
/// default talk range `4 * (16 + 16)`).
const TALK_OFFSET: f32 = 48.0;
/// Ticks of continuous user control before the explorer moves the player.
const CONTROL_SETTLE: u64 = 30;

/// Entity `register` uses for an entry's function with `tag`.
/// A floor point (x, z) kept on the walkmesh: the point itself when a
/// triangle contains it, else the nearest triangle centroid, so walkmesh
/// queries (`BGI` triangle polls) keep answering for the placed player.
fn snap_to_walkmesh(
    mesh: Option<&tantalus_contracts::FieldNavigationMesh>,
    point: [f32; 2],
) -> [f32; 2] {
    let Some(mesh) = mesh else {
        return point;
    };
    let mut nearest: Option<(f32, [f32; 2])> = None;
    for triangle in &mesh.triangles {
        let corners: Vec<[f32; 2]> = triangle
            .vertices
            .iter()
            .filter_map(|index| mesh.vertices.get(*index as usize))
            .map(|vertex| {
                [
                    (vertex.x + triangle.offset.x) as f32,
                    (vertex.z + triangle.offset.z) as f32,
                ]
            })
            .collect();
        let [a, b, c] = corners[..] else {
            continue;
        };
        let side = |p: [f32; 2], q: [f32; 2]| {
            (q[0] - p[0]) * (point[1] - p[1]) - (q[1] - p[1]) * (point[0] - p[0])
        };
        let (ab, bc, ca) = (side(a, b), side(b, c), side(c, a));
        if (ab >= 0.0 && bc >= 0.0 && ca >= 0.0) || (ab <= 0.0 && bc <= 0.0 && ca <= 0.0) {
            return point;
        }
        let center = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0];
        let distance = (center[0] - point[0]).powi(2) + (center[1] - point[1]).powi(2);
        if nearest.is_none_or(|(best, _)| distance < best) {
            nearest = Some((distance, center));
        }
    }
    nearest.map_or(point, |(_, center)| center)
}

/// World heights (`pos[1]`, the negated BGI height) of every walkmesh
/// triangle under `point`, one per floor that covers it.
fn walkmesh_heights(
    mesh: Option<&tantalus_contracts::FieldNavigationMesh>,
    point: [f32; 2],
) -> Vec<f32> {
    let Some(mesh) = mesh else {
        return Vec::new();
    };
    let mut heights = Vec::new();
    for triangle in &mesh.triangles {
        let corners: Vec<[f32; 3]> = triangle
            .vertices
            .iter()
            .filter_map(|index| mesh.vertices.get(*index as usize))
            .map(|vertex| {
                [
                    (vertex.x + triangle.offset.x) as f32,
                    -((vertex.y + triangle.offset.y) as f32),
                    (vertex.z + triangle.offset.z) as f32,
                ]
            })
            .collect();
        let [a, b, c] = corners[..] else {
            continue;
        };
        let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
        if det == 0.0 {
            continue;
        }
        let wa = ((b[2] - c[2]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[2])) / det;
        let wb = ((c[2] - a[2]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[2])) / det;
        let wc = 1.0 - wa - wb;
        if wa >= 0.0 && wb >= 0.0 && wc >= 0.0 {
            heights.push(wa * a[1] + wb * b[1] + wc * c[1]);
        }
    }
    heights
}

/// The walkmesh height under `point` nearest `current`, so a walk stays on
/// its floor where floors overlap. With no height set yet (32768,
/// `POS_COMMAND_DEFAULTY`), the first floor found.
fn walkmesh_height(
    mesh: Option<&tantalus_contracts::FieldNavigationMesh>,
    point: [f32; 2],
    current: f32,
) -> Option<f32> {
    let heights = walkmesh_heights(mesh, point);
    if current.abs() >= 32000.0 {
        return heights.first().copied();
    }
    heights
        .into_iter()
        .min_by(|a, b| (a - current).abs().total_cmp(&(b - current).abs()))
}

/// One step of a walk over the walkmesh toward `target`: the centre of the
/// next triangle on the shortest triangle path, with its height, and
/// whether the actor has arrived. The walk starts on the triangle nearest
/// the actor in 3D, so it keeps to the floor the actor stands on and
/// changes floors only by stairs or ramps (map 154 region 8 leaves to 158
/// from the ground floor and to 153 from the upper one). `None` when the
/// actor is off the mesh, as on map 115's tower top.
fn walkmesh_step(
    mesh: Option<&tantalus_contracts::FieldNavigationMesh>,
    position: [f32; 3],
    target: [f32; 2],
) -> Option<([f32; 3], bool)> {
    const OFF_MESH: f32 = 300.0;
    type Corners = [[f32; 3]; 3];
    let mesh = mesh?;
    let corners: Vec<Corners> = mesh
        .triangles
        .iter()
        .filter_map(|triangle| {
            let mut out = [[0.0f32; 3]; 3];
            for (slot, index) in out.iter_mut().zip(triangle.vertices) {
                let vertex = mesh.vertices.get(index as usize)?;
                *slot = [
                    (vertex.x + triangle.offset.x) as f32,
                    -((vertex.y + triangle.offset.y) as f32),
                    (vertex.z + triangle.offset.z) as f32,
                ];
            }
            Some(out)
        })
        .collect();
    let centre = |t: &Corners| {
        [
            (t[0][0] + t[1][0] + t[2][0]) / 3.0,
            (t[0][1] + t[1][1] + t[2][1]) / 3.0,
            (t[0][2] + t[1][2] + t[2][2]) / 3.0,
        ]
    };
    let covers = |t: &Corners, x: f32, z: f32| -> Option<f32> {
        let [a, b, c] = *t;
        let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
        if det == 0.0 {
            return None;
        }
        let wa = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
        let wb = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
        let wc = 1.0 - wa - wb;
        (wa >= 0.0 && wb >= 0.0 && wc >= 0.0).then(|| wa * a[1] + wb * b[1] + wc * c[1])
    };
    // The triangle the actor stands on: one under it at its height, else
    // the nearest in 3D (a doorway just past the mesh edge).
    let unset = position[1].abs() >= 32000.0;
    let (start, distance) = corners
        .iter()
        .enumerate()
        .map(|(index, t)| {
            let distance = match covers(t, position[0], position[2]) {
                Some(_) if unset => 0.0,
                Some(height) => (height - position[1]).abs(),
                None => {
                    // Distance to the nearest point of the triangle's
                    // edges, measured in 3D.
                    let mut best = f32::MAX;
                    for (i, j) in [(0, 1), (1, 2), (2, 0)] {
                        let (a, b) = (t[i], t[j]);
                        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                        let length = d[0] * d[0] + d[2] * d[2];
                        let along = if length == 0.0 {
                            0.0
                        } else {
                            (((position[0] - a[0]) * d[0] + (position[2] - a[2]) * d[2]) / length)
                                .clamp(0.0, 1.0)
                        };
                        let p = [
                            a[0] + d[0] * along,
                            a[1] + d[1] * along,
                            a[2] + d[2] * along,
                        ];
                        let dy = if unset { 0.0 } else { p[1] - position[1] };
                        let distance = ((p[0] - position[0]).powi(2)
                            + dy.powi(2)
                            + (p[2] - position[2]).powi(2))
                        .sqrt();
                        best = best.min(distance);
                    }
                    best
                }
            };
            (index, distance)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    if distance > OFF_MESH {
        return None;
    }
    // Triangles sharing an edge (two corners at the same world point).
    type Key = (i32, i32, i32);
    let key = |p: [f32; 3]| -> Key {
        (
            p[0].round() as i32,
            p[1].round() as i32,
            p[2].round() as i32,
        )
    };
    let mut edges: BTreeMap<(Key, Key), Vec<usize>> = BTreeMap::new();
    for (index, t) in corners.iter().enumerate() {
        for (i, j) in [(0, 1), (1, 2), (2, 0)] {
            let (a, b) = (key(t[i]), key(t[j]));
            edges
                .entry(if a < b { (a, b) } else { (b, a) })
                .or_default()
                .push(index);
        }
    }
    let mut neighbours = vec![Vec::new(); corners.len()];
    for shared in edges.values() {
        for &a in shared {
            for &b in shared {
                if a != b {
                    neighbours[a].push(b);
                }
            }
        }
    }
    // Breadth-first to the nearest reachable triangle under the target;
    // when none is reachable, to the reachable triangle closest to it.
    let mut previous = vec![usize::MAX; corners.len()];
    previous[start] = start;
    let mut queue = std::collections::VecDeque::from([start]);
    let mut goal = None;
    let mut closest = (f32::MAX, start);
    while let Some(index) = queue.pop_front() {
        if covers(&corners[index], target[0], target[1]).is_some() {
            goal = Some(index);
            break;
        }
        let c = centre(&corners[index]);
        let distance = (c[0] - target[0]).powi(2) + (c[2] - target[1]).powi(2);
        if distance < closest.0 {
            closest = (distance, index);
        }
        for &next in &neighbours[index] {
            if previous[next] == usize::MAX {
                previous[next] = index;
                queue.push_back(next);
            }
        }
    }
    let goal = goal.unwrap_or(closest.1);
    if goal == start {
        return Some(match covers(&corners[start], target[0], target[1]) {
            Some(height) => ([target[0], height, target[1]], true),
            // Closest reachable point; nothing nearer to walk to.
            None => (centre(&corners[start]), true),
        });
    }
    let mut step = goal;
    while previous[step] != start {
        step = previous[step];
    }
    Some((centre(&corners[step]), false))
}

/// `MJPOS` (EventEngine.DoEventCode.cs:2200): `position` projected onto
/// the region's first edge, clamped to it. Exit scripts compare the
/// player's facing with the direction to this point.
fn exit_point(vertices: &[[i16; 2]], position: [f32; 3]) -> Option<[f32; 2]> {
    let [first, second] = [vertices.first()?, vertices.get(1)?];
    let (x1, z1) = (f32::from(first[0]), f32::from(first[1]));
    let (dx, dz) = (f32::from(second[0]) - x1, f32::from(second[1]) - z1);
    let length = dx * dx + dz * dz;
    if length == 0.0 {
        return Some([x1, z1]);
    }
    let along = ((dx * (position[0] - x1) + dz * (position[2] - z1)) / length).clamp(0.0, 1.0);
    Some([x1 + along * dx, z1 + along * dz])
}

/// Facing (member 3, 256 steps: 0 south, 64 west, 128 north, 192 east,
/// the `Angle2` convention) from `position` toward `target`.
fn facing_toward(position: [f32; 3], target: [f32; 2]) -> Option<i16> {
    let (dx, dz) = (target[0] - position[0], target[1] - position[2]);
    if dx.abs() < 1.0 && dz.abs() < 1.0 {
        return None;
    }
    let turns = (-dx).atan2(-dz) / std::f32::consts::TAU;
    Some(((turns * 256.0).round() as i32 & 255) as i16)
}

/// A point inside `region`: the vertex average when `IsInQuad` holds
/// there, else the centre of the first of its triangles that contains its
/// own centre (map 164 region 2's average falls outside it).
fn inside_point(region: &tantalus_game::field::region::Region) -> [f32; 2] {
    let vertices = &region.vertices;
    let n = vertices.len().max(1) as f32;
    let (x, z) = vertices.iter().fold((0.0, 0.0), |(x, z), [vx, vz]| {
        (x + f32::from(*vx), z + f32::from(*vz))
    });
    let average = [x / n, z / n];
    if region.contains(average[0], average[1]) {
        return average;
    }
    (0..vertices.len())
        .map(|i| {
            let corner = |k: usize| vertices[(i + k) % vertices.len()].map(f32::from);
            let (a, b, c) = (corner(0), corner(1), corner(2));
            [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0]
        })
        .find(|point| region.contains(point[0], point[1]))
        .unwrap_or(average)
}

struct ChainOptions {
    start: i32,
    max_fields: usize,
    ticks: u64,
    confirm_after: u64,
    explore: bool,
    /// Cycle synthetic direction bits so the controlled actor walks and
    /// faces; scenes that gate on player facing (map 100) then advance.
    walk_demo: bool,
    /// Directory for per-field actor tracks (`NNN_MAP.jsonl`): a header
    /// with the background name, then actor positions every two ticks, for
    /// `tantalus-client view-track`.
    actor_track: Option<PathBuf>,
    /// World map stand-in: on `ChangeWorldMap WORLD`, the first unused
    /// entry for WORLD (from SCENARIO on) picks the world location the
    /// party enters, and the world script's own exit code for that location
    /// names the next field.
    world_exits: Vec<(i32, i32, i32)>,
    stats: BTreeMap<i32, tantalus_game::field::CharacterStats>,
    text: Option<FieldText>,
    /// Scripted answers for choice windows by (field map, text id); 0 when
    /// absent, the reference's default cursor position.
    choices: BTreeMap<(i32, u16, i32), i32>,
    /// `FF9_Data` directory for the FMV movie inventory (`StreamingAssets/ma`).
    game_data: Option<PathBuf>,
    /// Level tables for experience growth (base stats and leveling rows).
    growth_tables: Option<(
        BTreeMap<i32, tantalus_game::new_game::BaseStats>,
        Vec<tantalus_game::new_game::LevelRow>,
    )>,
    /// `InitialItems.csv`: the new-game inventory.
    initial_items: BTreeMap<i32, i32>,
    /// Vanilla item-table legacy ids for the reward-item identity mapping.
    item_ids: Option<Vec<i32>>,
    /// Free-roam route: while MAP's first unused entry is DEST, the explorer
    /// only enters exit regions that lead to DEST; the entry is used up when
    /// the field leaves for DEST. Regions whose tag 2 holds no
    /// constant `ChangeField` stay allowed. An entry with `@SCENARIO` waits
    /// until the scenario counter reaches that value.
    route: Vec<(i32, i32, i32)>,
    /// Objects the explorer never talks to, by (field map, entry): optional
    /// minigames that hold the field until played (map 103 entry 8, jump rope).
    skip_talk: BTreeSet<(i32, usize)>,
    /// Pad bits held for a whole field, by field map (`KeyState(Held)`
    /// polls such as map 115's ladder climb, which reads up = 16).
    hold: BTreeMap<i32, i32>,
    /// `EventEngineUtils.eventIDToFBGID`: field map number to background
    /// name, whose `<name>.bgi` walkmesh backs `BGI` triangle queries.
    backgrounds: BTreeMap<i32, String>,
    /// Floor positions (x, z) the scripted player visits in order, by field
    /// map, once its regions and talks are done: fields that wait for the
    /// player to walk somewhere (map 116's roof waits for Vivi's x <= 900).
    waypoints: BTreeMap<i32, Vec<[f32; 2]>>,
    /// Fight each encounter on the legacy battle machine (`--battles`).
    battles: Option<tantalus_tools::field_battle::FieldBattles>,
}

/// `gEventGlobal` Int16 0, the scenario counter that orders the story.
fn scenario_counter(world: &tantalus_game::FieldWorld) -> i32 {
    let bytes = world.global_memory.bytes();
    i32::from(bytes[0]) | (i32::from(bytes[1] as i8) << 8)
}

/// Destination of a region's exit: the first `ChangeField` with a constant
/// operand in the entry's tag-2 function.
fn exit_destination(functions: &[(u16, EventProgram)]) -> Option<i32> {
    let (_, program) = functions.iter().find(|(tag, _)| *tag == 2)?;
    program
        .instructions
        .values()
        .find_map(|instruction| match &instruction.kind {
            InstructionKind::ChangeField(Expression(ops)) => match ops.as_slice() {
                [ExpressionOp::Constant(map), ExpressionOp::End] => Some(*map),
                _ => None,
            },
            _ => None,
        })
}

/// Every constant `ChangeField` destination in a region's tag-2 function:
/// one region can branch (map 154 region 8 leaves to 153 or 158 by height).
fn exit_destinations(functions: &[(u16, EventProgram)]) -> Vec<i32> {
    let Some((_, program)) = functions.iter().find(|(tag, _)| *tag == 2) else {
        return Vec::new();
    };
    let mut destinations: Vec<i32> = program
        .instructions
        .values()
        .filter_map(|instruction| match &instruction.kind {
            InstructionKind::ChangeField(Expression(ops)) => match ops.as_slice() {
                [ExpressionOp::Constant(map), ExpressionOp::End] => Some(*map),
                _ => None,
            },
            _ => None,
        })
        .collect();
    destinations.dedup();
    destinations
}

fn chain(
    bundle: &Path,
    db: &BTreeMap<i32, String>,
    atan: Option<Vec<u8>>,
    options: ChainOptions,
) -> Result<(), String> {
    let ChainOptions {
        start,
        max_fields,
        ticks,
        confirm_after,
        explore,
        walk_demo,
        actor_track,
        world_exits,
        stats,
        text,
        choices,
        game_data,
        growth_tables,
        initial_items,
        item_ids,
        route,
        skip_talk,
        hold,
        backgrounds,
        waypoints,
        battles: field_battles,
    } = options;
    let battle_base: BTreeMap<i32, tantalus_game::new_game::BaseStats> = growth_tables
        .as_ref()
        .map(|(base, _)| base.clone())
        .unwrap_or_default();
    // Field bundles sit beside the event bundle (`StreamingAssets`).
    let field_bundles: Vec<PathBuf> = (11..=19)
        .map(|n| {
            bundle
                .parent()
                .unwrap_or(Path::new("."))
                .join(format!("p0data{n}.bin"))
        })
        .collect();
    let mut route_used = vec![false; route.len()];
    // Field visits so far: with no route entry, the explorer leaves through
    // the exit to the least visited field, so two fields whose first exits
    // lead to each other do not hand control back and forth forever.
    let mut visits: BTreeMap<i32, usize> = BTreeMap::new();
    // Where each (map, exit region) actually led. A region whose script
    // branches (map 153 region 25: 151 or 64 by height) is judged by what
    // it did, not by every destination in its code.
    let mut observed_exits: BTreeMap<(i32, usize), BTreeSet<i32>> = BTreeMap::new();
    // Talks across visits as (map, entry, scenario counter): talking to the
    // same object again at the same story point repeats what it said.
    let mut talked_before: BTreeSet<(i32, usize, i32)> = BTreeSet::new();
    let mut world = tantalus_game::FieldWorld::new();
    // Movie frame counts for the FMV service: scan the Ogg container of
    // `ma/FMV%03d.bytes` on first use. A failed or unknown file keeps the
    // as-if-skipped behavior.
    if let Some(game_data) = &game_data {
        // Movies live at the install root (`StreamingAssets/ma`), two
        // levels up from `x64/FF9_Data`.
        let movies = game_data
            .parent()
            .and_then(|parent| parent.parent())
            .map(|root| root.join("StreamingAssets/ma"))
            .unwrap_or_else(|| game_data.join("StreamingAssets/ma"));
        let cache = std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::<
            (i32, i32),
            Option<u32>,
        >::new()));
        world.fmv_frame_counts = Some(std::sync::Arc::new(move |disc, number| {
            let key = (disc, number);
            if let Some(hit) = cache.lock().unwrap().get(&key) {
                return *hit;
            }
            let resolved = (0..4u8)
                .map(|suffix| {
                    if suffix == 0 {
                        movies.join(format!("FMV{number:03}.bytes"))
                    } else {
                        movies.join(format!(
                            "FMV{number:03}{}.bytes",
                            (b'A' + suffix - 1) as char
                        ))
                    }
                })
                .find(|path| path.is_file())
                .and_then(|path| {
                    let report = tantalus_assets::pc::ogg::scan_ogg_file(&path, false).ok()?;
                    report
                        .streams
                        .iter()
                        .find(|stream| stream.codec.name() == "theora")?
                        .frame_count
                        .map(|count| u32::try_from(count).unwrap_or(u32::MAX))
                });
            cache.lock().unwrap().insert(key, resolved);
            resolved
        }));
    }
    // FF9Play_Init: every character has a player record from new game on.
    world.character_stats = stats;
    world.items = initial_items;
    if let Some(ids) = item_ids {
        world.item_id_map = Some(tantalus_game::field::world::default_item_id_map(ids));
    }
    if let Some((base, levels)) = growth_tables {
        world.level_growth = Some(std::sync::Arc::new(move |character, total_exp| {
            let base = base.get(&character)?;
            let level = levels
                .iter()
                .enumerate()
                .filter(|(_, row)| u64::from(row.experience) <= total_exp)
                .map(|(index, _)| index + 1)
                .max()
                .unwrap_or(1)
                .min(levels.len());
            tantalus_game::new_game::build_stats(*base, &levels, level)
        }));
    }
    if let Some(table) = atan {
        world.provide_atan_table(table);
    }
    let mut map = start;
    if let Some(dir) = &actor_track {
        std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    }
    let mut world_exits_used = vec![false; world_exits.len()];
    for field_number in 0..max_fields {
        let Some(name) = db.get(&map) else {
            println!(
                "{}",
                serde_json::json!({"map": map, "stop": "map number not in EventDB"})
            );
            return Ok(());
        };
        let archive = format!("{name}.eb");
        let binary = match load_event(bundle, &archive) {
            Ok(binary) => binary,
            Err(error) => {
                println!(
                    "{}",
                    serde_json::json!({"map": map, "archive": archive, "stop": error})
                );
                return Ok(());
            }
        };
        let (pool, decode_errors) = decode_pool(&binary)?;
        let exit_choices: BTreeMap<usize, Vec<i32>> = (0..pool.len())
            .map(|entry| (entry, exit_destinations(&pool[entry])))
            .filter(|(_, destinations)| !destinations.is_empty())
            .collect();
        let exits: BTreeMap<usize, i32> = (0..pool.len())
            .filter_map(|entry| Some((entry, exit_destination(&pool[entry])?)))
            .collect();
        // The first unused route entry for this map; it is used up when the
        // field leaves through its destination.
        let scenario_now = scenario_counter(&world);
        let route_index = route
            .iter()
            .enumerate()
            .position(|(index, (from, _, after))| {
                *from == map && !route_used[index] && scenario_now >= *after
            });
        let route_target = route_index.map(|index| route[index].1);
        *visits.entry(map).or_default() += 1;
        world.field_map_no = map;
        let held = hold.get(&map).copied().unwrap_or(0);
        // EventEngine.InitEvents zeroes mapvar for every field (EventEngine.cs:556).
        world.map_memory.clear();
        world.begin_field();
        let navigation_status = match backgrounds.get(&map) {
            Some(name) => match load_navigation(&field_bundles, name) {
                Ok(mesh) => {
                    let status = format!("{name}: {} triangles", mesh.triangles.len());
                    world.provide_navigation(mesh);
                    status
                }
                Err(error) => {
                    world.clear_navigation();
                    error
                }
            },
            None => {
                world.clear_navigation();
                "no background name".to_owned()
            }
        };
        let mut choice_texts = BTreeSet::new();
        let mut plain_texts: BTreeMap<u16, String> = BTreeMap::new();
        let text_status = match &text {
            Some(text) => match text.info(map) {
                Ok(info) => {
                    let status = format!(
                        "{} texts with signals, {} with choices",
                        info.signals.len(),
                        info.choices.len()
                    );
                    world.dialog_signals = info.signals;
                    choice_texts = info.choices;
                    plain_texts = info.plain;
                    status
                }
                Err(error) => {
                    world.dialog_signals.clear();
                    error
                }
            },
            None => "not loaded".to_owned(),
        };
        let mut director = tantalus_tools::field_session::new_director(world.clone(), &pool, held);
        let mut scheduler = EntryScheduler::start(&mut director, &pool)?;
        // Scripted player: with --confirm-after N, a window open for N ticks
        // is closed as if confirm were pressed (0 disables).
        let mut open_since: BTreeMap<u8, u64> = BTreeMap::new();
        let mut confirms = 0usize;
        let mut choices_made: Vec<(u16, i32)> = Vec::new();
        // Scripted player (--explore): while user control is on, step the
        // controlled actor into each region that has a tag-2 function, run
        // that function once (the reference `Request(quad, 1, 2)` from
        // `EventCollision.CollisionRequest`), and press Confirm every
        // CONFIRM_PERIOD ticks. Placing the actor at the region centroid
        // stands in for walking there.
        if walk_demo {
            // Synthetic demo mapping (not the pad layout): 0x01 up, 0x02
            // down, 0x04 left, 0x08 right. 64 ticks per leg, like the probe.
            director.set_walk_bits(Some((0x01, 0x02, 0x04, 0x08)));
        }
        let mut triggered: BTreeSet<usize> = BTreeSet::new();
        let mut last_exit_region: Option<usize> = None;
        let scenario_at_start = scenario_counter(&world);
        let mut talked: BTreeSet<usize> = skip_talk
            .iter()
            .filter(|(skip_map, _)| *skip_map == map)
            .map(|(_, entry)| *entry)
            .chain(
                talked_before
                    .iter()
                    .filter(|(talk_map, _, scenario)| {
                        *talk_map == map && *scenario == scenario_at_start
                    })
                    .map(|(_, entry, _)| *entry),
            )
            .collect();
        let mut talk_dwell: BTreeMap<usize, u64> = BTreeMap::new();
        let field_waypoints = waypoints.get(&map).cloned().unwrap_or_default();
        let mut floors_tried: BTreeMap<usize, Vec<f32>> = BTreeMap::new();
        let mut exit_runs: BTreeMap<usize, (u32, usize)> = BTreeMap::new();
        let mut waypoint_index = 0usize;
        let mut waypoint_dwell = 0u64;
        let mut talk_scenario = scenario_at_start;
        let skipped: BTreeSet<usize> = skip_talk
            .iter()
            .filter(|(skip_map, _)| *skip_map == map)
            .map(|(_, entry)| *entry)
            .collect();
        let mut talks: Vec<(u64, usize)> = Vec::new();
        if explore {
            director.set_confirm_mask(Some(CONFIRM));
        }
        let mut region_triggers = 0usize;
        let mut dwell_ticks: BTreeMap<usize, u64> = BTreeMap::new();
        let mut presses = 0usize;
        let mut pressed = false;
        let mut operations = 0u64;
        let mut ticks_run = 0u64;
        let mut error = None;
        let mut transition = None;
        let mut world_map: Option<i32> = None;
        let mut encounters = 0usize;
        let mut battles: Vec<i32> = Vec::new();
        let mut fights: Vec<tantalus_tools::field_battle::FightReport> = Vec::new();
        // (tick, user control, Map byte 24) whenever user control changes.
        let mut control_changes: Vec<(u64, bool, u8)> = Vec::new();
        // Each text a window shows, in order, as (tick, text id).
        let mut texts_shown: Vec<(u64, u16)> = Vec::new();
        let mut last_texts: BTreeMap<u8, u16> = BTreeMap::new();
        let mut last_control = director.world().user_control;
        let mut track = match &actor_track {
            Some(dir) => {
                let path = dir.join(format!("{field_number:03}_{map}.jsonl"));
                let mut file = std::io::BufWriter::new(
                    std::fs::File::create(&path)
                        .map_err(|error| format!("{}: {error}", path.display()))?,
                );
                let header = serde_json::json!({
                    "map": map,
                    "field": backgrounds.get(&map),
                    "scenario_counter": scenario_counter(director.world()),
                });
                use std::io::Write as _;
                writeln!(file, "{header}").map_err(|error| error.to_string())?;
                Some(file)
            }
            None => None,
        };
        while ticks_run < ticks {
            if let Some(file) = track.as_mut()
                && ticks_run.is_multiple_of(2)
            {
                let world = director.world();
                let actors: Vec<serde_json::Value> = world
                    .actors
                    .iter()
                    .map(|(id, actor)| {
                        serde_json::json!([
                            id.get(),
                            actor.position[0].round(),
                            actor.position[1].round(),
                            actor.position[2].round(),
                            actor.facing
                        ])
                    })
                    .collect();
                let line = serde_json::json!({
                    "tick": ticks_run,
                    "player": world.player().map(|player| player.get()),
                    "control": world.user_control,
                    "actors": actors,
                });
                use std::io::Write as _;
                writeln!(file, "{line}").map_err(|error| error.to_string())?;
            }
            for (window, text) in &director.world().window_texts {
                if last_texts.insert(*window, *text) != Some(*text) {
                    texts_shown.push((ticks_run, *text));
                }
            }
            let control = director.world().user_control;
            if control != last_control {
                let stage = director.world().map_memory.bytes()[24];
                control_changes.push((ticks_run, control, stage));
                last_control = control;
            }
            if walk_demo {
                let phase = (ticks_run / 64) % 4;
                let direction = [0x04u8, 0x01, 0x08, 0x02][phase as usize];
                director.world_mut().set_input(i32::from(direction));
            }
            match director.step() {
                Ok(step) => {
                    ticks_run += 1;
                    operations += step.operations;
                    scheduler.start_created(&mut director, &pool, &step)?;
                    encounters += step.encounters.len();
                    // No battle engine yet: each scripted battle is skipped
                    // and the field resumes on the next tick, as after a win.
                    battles.extend(step.encounters.iter().map(|request| request.battle));
                    // With `--battles`, fight each one first; the field
                    // still resumes as after a win.
                    if let Some(field_battles) = &field_battles {
                        let world = director.world();
                        let party: Vec<tantalus_tools::field_battle::PartyFighter> = world
                            .party
                            .iter()
                            .filter_map(|character| {
                                let base = *battle_base.get(character)?;
                                let stats = world.character_stats.get(character)?;
                                Some(tantalus_tools::field_battle::PartyFighter {
                                    character: *character,
                                    hp: stats.current_hp.max(0) as u32,
                                    max_hp: stats.max_hp.max(0) as u32,
                                    mp: stats.current_mp.max(0) as u32,
                                    max_mp: stats.max_mp.max(0) as u32,
                                    base,
                                    globals:
                                        tantalus_tools::field_battle::PartyGlobals::from_globals(
                                            world.global_memory.bytes(),
                                        ),
                                    row: u8::from(!world.back_row.contains(character)),
                                })
                            })
                            .collect();
                        for request in &step.encounters {
                            let items = director.world().items.clone();
                            let report = field_battles.fight_with_items(request, &party, &items);
                            tantalus_tools::field_battle::save_party(
                                &report,
                                &mut director.world_mut().character_stats,
                            );
                            tantalus_tools::field_battle::pay_rewards(
                                &report,
                                director.world_mut(),
                            );
                            fights.push(report);
                        }
                        // Back on the same field after a battle,
                        // `EventEngine.StartEvents` restores the saved
                        // context and requests the main object's function 10
                        // at level 0 (`EventEngine.cs:598`): map 202 sets
                        // scenario 2200 there after Baku's fight.
                        if !step.encounters.is_empty() {
                            tantalus_tools::field_session::resume_after_battle(
                                &mut director,
                                &pool,
                            )?;
                        }
                    }
                    // Talk (EventCollision): confirm next to an object starts
                    // its tag 3. The scripted player talks to each once.
                    if explore {
                        // A story step (scenario counter change) can give an
                        // object new lines or a new use (map 115's ladder),
                        // so the explorer may talk to it again.
                        let scenario = scenario_counter(director.world());
                        if scenario != talk_scenario {
                            talk_scenario = scenario;
                            talked.retain(|entry| skipped.contains(entry));
                            talk_dwell.clear();
                        }
                        // One confirm starts one talk (`EventCollision`
                        // requests a single object), and none while another
                        // talk function is still running.
                        let states = director.entry_states();
                        let mut talk_busy = (0..pool.len()).any(|entry| {
                            function_entity(&pool, entry, 3).is_some_and(|entity| {
                                states.iter().any(|(e, _, status, _)| {
                                    *e == entity && *status != VmStatus::Completed
                                })
                            })
                        });
                        for trigger in &step.talk_triggers {
                            let entry = trigger.get() as usize;
                            if talk_busy || entry >= pool.len() || talked.contains(&entry) {
                                continue;
                            }
                            // Rerun a finished talk function from its start.
                            if let Some(entity) = function_entity(&pool, entry, 3)
                                && director.entry_states().iter().any(|(e, _, status, _)| {
                                    *e == entity && *status == VmStatus::Completed
                                })
                            {
                                director.remove_entry(entity);
                            }
                            if register(&mut director, &pool, entry, Some(3))?.is_some() {
                                talk_busy = true;
                                talked.insert(entry);
                                talks.push((ticks_run, entry));
                                talked_before.insert((
                                    map,
                                    entry,
                                    scenario_counter(director.world()),
                                ));
                            }
                        }
                    }
                    scheduler.run_step_scripts(&mut director, &pool, &step)?;
                }
                Err(failure) => {
                    error = Some(failure.to_string());
                    break;
                }
            }
            scheduler.settle(&mut director, &pool)?;
            // A confirm press lasts one tick.
            if pressed {
                director.world_mut().set_input(held);
                pressed = false;
            }
            // Through a cutscene the player keeps tapping confirm; scenes
            // that skip ahead on `KeyOn` (map 150 entry 23) wait for a
            // press that comes after the line has shown.
            if confirm_after > 0
                && !director.world().user_control
                && ticks_run > 0
                && ticks_run.is_multiple_of(CONFIRM_PERIOD)
            {
                let quiet_since = texts_shown.last().map_or(0, |(tick, _)| *tick);
                let stalled = ticks_run.saturating_sub(quiet_since) >= STALL_TICKS;
                let prompt = director
                    .world()
                    .window_texts
                    .values()
                    .filter_map(|text| {
                        PROMPT_KEYS
                            .iter()
                            .find(|(on, id, _)| *on == map && id == text)
                            .map(|(_, _, mask)| *mask)
                    })
                    .fold(0, |keys, mask| keys | mask);
                let keys = if prompt != 0 {
                    prompt
                } else if stalled {
                    CONFIRM | CANCEL_SPECIAL
                } else {
                    CONFIRM
                };
                director.world_mut().set_input(keys | held);
                pressed = true;
                presses += 1;
            }
            if confirm_after > 0 {
                let open: Vec<u8> = director
                    .world()
                    .windows
                    .iter()
                    .filter(|(_, open)| **open)
                    .map(|(window, _)| *window)
                    .collect();
                open_since.retain(|window, _| open.contains(window));
                for window in open {
                    let since = *open_since.entry(window).or_insert(ticks_run);
                    if ticks_run - since >= confirm_after {
                        let shown = director.world().window_texts.get(&window).copied();
                        // A button prompt closes itself once its key is
                        // pressed (map 64 entry 3); answer it instead.
                        let prompt = PROMPT_KEYS
                            .iter()
                            .find(|(on, id, _)| *on == map && Some(*id) == shown)
                            .map(|(_, _, mask)| *mask);
                        if let Some(mask) = prompt {
                            director.world_mut().set_input(mask | held);
                            pressed = true;
                            continue;
                        }
                        if let Some(text) = shown.filter(|text| choice_texts.contains(text)) {
                            // The answer set for the latest story step
                            // reached: map 202's Baku starts his fight on
                            // "Anytime!" only from scenario 2100.
                            let scenario = scenario_counter(director.world());
                            let answer = choices
                                .range((map, text, i32::MIN)..=(map, text, scenario))
                                .next_back()
                                .map_or(0, |(_, answer)| *answer);
                            director.world_mut().dialog_choice = answer;
                            choices_made.push((text, answer));
                        }
                        director.world_mut().close_window(window);
                        open_since.remove(&window);
                        confirms += 1;
                        // Closing a window is a confirm press, which
                        // scripts polling `KeyOn` also see (map 150 entry
                        // 23 waits for one after text 229).
                        director.world_mut().set_input(CONFIRM | held);
                        pressed = true;
                    }
                }
            }
            if explore {
                let world = director.world();
                // A player cannot reach a region within a few frames of
                // getting control; scenes often grant it for one or two
                // ticks while they start. Act only once control has held
                // for CONTROL_SETTLE ticks.
                let settled = world.user_control
                    && control_changes.last().is_none_or(|(tick, _, _)| {
                        ticks_run.saturating_sub(*tick) >= CONTROL_SETTLE
                    });
                let controlled = settled
                    .then_some(world.control_actor)
                    .flatten()
                    .and_then(|actor| Some((actor, world.actors.get(&actor)?.position)));
                if let Some((actor, position)) = controlled {
                    // The one exit the explorer heads for: the routed
                    // destination, else the least visited one.
                    let chosen_exit = world
                        .regions
                        .keys()
                        .map(|region| region.get() as usize)
                        .filter(|entry| !triggered.contains(entry))
                        .filter_map(|entry| {
                            let observed = observed_exits.get(&(map, entry));
                            let dests: Vec<i32> = match observed {
                                Some(seen) if !seen.is_empty() => seen.iter().copied().collect(),
                                _ => exit_choices.get(&entry)?.clone(),
                            };
                            Some((entry, dests))
                        })
                        .filter(|(_, dests)| {
                            route_target.is_none_or(|target| dests.contains(&target))
                        })
                        .min_by_key(|(entry, dests)| {
                            let least = dests
                                .iter()
                                .map(|dest| visits.get(dest).copied().unwrap_or(0))
                                .min()
                                .unwrap_or(0);
                            (least, *entry)
                        })
                        .map(|(entry, _)| entry);
                    let has_tag2 = |region: &EntityId| {
                        let entry = region.get() as usize;
                        let routed = !exits.contains_key(&entry) || chosen_exit == Some(entry);
                        routed && entry < pool.len() && pool[entry].iter().any(|(tag, _)| *tag == 2)
                    };
                    let hit = tantalus_game::field::region::regions_at(
                        &world.regions,
                        position[0],
                        position[2],
                    )
                    .into_iter()
                    .find(|region| {
                        has_tag2(region) && !triggered.contains(&(region.get() as usize))
                    })
                    .map(|region| region.get() as usize);
                    // Targets in order: regions that do not leave the field,
                    // then each object with a talk function (tag 3), then
                    // the exits the route allows.
                    let pending_region = |exit: bool| {
                        world
                            .regions
                            .iter()
                            .find(|(region, _)| {
                                let entry = region.get() as usize;
                                has_tag2(region)
                                    && !triggered.contains(&entry)
                                    && exits.contains_key(&entry) == exit
                            })
                            .map(|(_, region)| inside_point(region))
                    };
                    let pending_talk = (0..pool.len())
                        .filter(|entry| {
                            *entry as u64 != actor.get()
                                && !talked.contains(entry)
                                && pool[*entry].iter().any(|(tag, _)| *tag == 3)
                        })
                        .find_map(|entry| {
                            let state = world.actors.get(&EntityId::new(entry as u64))?;
                            Some((entry, [state.position[0] + TALK_OFFSET, state.position[2]]))
                        });
                    let talk_target = pending_talk.map(|(entry, _)| entry);
                    let waypoint = field_waypoints
                        .get(waypoint_index)
                        .map(|point| snap_to_walkmesh(world.navigation(), *point));
                    let next = pending_region(false)
                        .or(pending_talk.map(|(_, at)| at))
                        .or(waypoint)
                        .or_else(|| pending_region(true));
                    let talking = pending_region(false).is_none() && talk_target.is_some();
                    let walking = pending_region(false).is_none()
                        && talk_target.is_none()
                        && waypoint.is_some();
                    // A player leaving through an exit walks toward it, so
                    // exit scripts that check facing (map 150 entry 18) see
                    // the player facing the exit point.
                    let exit_facing = hit
                        .filter(|entry| exits.contains_key(entry))
                        .and_then(|entry| world.regions.get(&EntityId::new(entry as u64)))
                        .and_then(|region| exit_point(&region.vertices, position))
                        .and_then(|point| facing_toward(position, point));
                    match hit {
                        Some(entry) => {
                            if let (Some(facing), Some(state)) =
                                (exit_facing, director.world_mut().actors.get_mut(&actor))
                            {
                                state.facing = facing;
                            }
                            // Request(quad, 1, 2) every frame while inside: rerun
                            // the region function once the previous run finished.
                            let entity = function_entity(&pool, entry, 2);
                            let finished =
                                director.entry_states().iter().any(|(e, _, status, _)| {
                                    Some(*e) == entity && *status == VmStatus::Completed
                                });
                            if finished {
                                director.remove_entry(entity.expect("tag-2 function exists"));
                            }
                            if exits.contains_key(&entry) {
                                last_exit_region = Some(entry);
                            }
                            if register(&mut director, &pool, entry, Some(2))?.is_some() {
                                region_triggers += 1;
                                // An exit whose script takes control for a
                                // line and keeps the player here refuses for now
                                // (map 206 to the forest: "too tough on my
                                // own"); after a few, try another exit.
                                // Silent reruns (map 164's floor retries)
                                // do not count.
                                let (refusals, seen) =
                                    exit_runs.entry(entry).or_insert((0, control_changes.len()));
                                if control_changes.len() > *seen {
                                    *refusals += 1;
                                }
                                *seen = control_changes.len();
                                if exits.contains_key(&entry) && *refusals >= EXIT_REFUSALS {
                                    triggered.insert(entry);
                                }
                            }
                            let dwell = dwell_ticks.entry(entry).or_insert(0);
                            *dwell += 1;
                            if *dwell >= REGION_DWELL {
                                // Exit scripts can test the height (map 164's
                                // spiral tower leaves upward only from above
                                // 12000), so try each other floor under the
                                // region before giving up on it.
                                let mesh = director.world().navigation();
                                let tried = floors_tried.entry(entry).or_default();
                                tried.push(position[1]);
                                let untried = walkmesh_heights(mesh, [position[0], position[2]])
                                    .into_iter()
                                    .filter(|height| {
                                        tried.iter().all(|old| (old - height).abs() > 100.0)
                                    })
                                    .min_by(|a, b| a.total_cmp(b));
                                match (untried, director.world_mut().actors.get_mut(&actor)) {
                                    (Some(height), Some(state)) if exits.contains_key(&entry) => {
                                        state.position[1] = height;
                                        *dwell = 0;
                                    }
                                    _ => {
                                        triggered.insert(entry);
                                    }
                                }
                            }
                        }
                        None => {
                            // Only an actor standing on the walkmesh follows it;
                            // one a script put elsewhere (map 115's tower top)
                            // keeps its height.
                            let mesh = director.world().navigation();
                            // Jump straight to a target on the actor's floor;
                            // walk the mesh only to reach another floor.
                            let on_mesh = position[1].abs() >= 32000.0
                                || walkmesh_height(mesh, [position[0], position[2]], position[1])
                                    .is_some();
                            let same_floor = |[x, z]: [f32; 2]| {
                                walkmesh_height(mesh, [x, z], position[1]).filter(|height| {
                                    on_mesh || (height - position[1]).abs() < 100.0
                                })
                            };
                            let direct = next.and_then(same_floor);
                            // A target off the mesh (a talk spot, a region
                            // centre) keeps the plain jump.
                            let step = next
                                .filter(|_| direct.is_none())
                                .filter(|[x, z]| {
                                    walkmesh_height(mesh, [*x, *z], position[1]).is_some()
                                })
                                .and_then(|target| walkmesh_step(mesh, position, target));
                            let arrived = step.is_none_or(|(_, arrived)| arrived);
                            if let (Some([x, z]), Some(state)) =
                                (next, director.world_mut().actors.get_mut(&actor))
                            {
                                let [x, y, z] = match (direct, step) {
                                    (Some(height), _) => [x, height, z],
                                    (None, Some((point, _))) => point,
                                    (None, None) => [x, state.position[1], z],
                                };
                                if let Some(facing) = facing_toward(state.position, [x, z]) {
                                    state.facing = facing;
                                }
                                let walked = (x - state.position[0]).hypot(z - state.position[2]);
                                state.position = [x, y, z];
                                state.movement = None;
                                // The jump stands in for walking there, which
                                // runs the random-battle timer.
                                director.world_mut().player_walked(walked);
                            }
                            // Give up on an object that never answers (out of
                            // range, hidden, or not placed yet).
                            if walking && arrived {
                                waypoint_dwell += 1;
                                if waypoint_dwell >= WAYPOINT_DWELL {
                                    waypoint_dwell = 0;
                                    waypoint_index += 1;
                                }
                            }
                            if let (true, Some(entry)) = (talking, talk_target) {
                                let dwell = talk_dwell.entry(entry).or_insert(0);
                                *dwell += 1;
                                if *dwell >= TALK_DWELL {
                                    talked.insert(entry);
                                }
                            }
                        }
                    }
                    if ticks_run.is_multiple_of(CONFIRM_PERIOD) {
                        director.world_mut().set_input(CONFIRM | held);
                        pressed = true;
                        presses += 1;
                    }
                }
            }
            // `WORLD` (ChangeWorldMap): the field hands off to the world
            // map; the chain has no world map yet, so it ends here.
            if let Some(destination) = director.world_mut().take_world_transitions().first() {
                world_map = Some(i32::from(*destination));
                break;
            }
            let transitions = director.world_mut().take_field_transitions();
            if let Some(next) = transitions.first() {
                transition = Some(i32::from(*next));
                if let Some(entry) = last_exit_region {
                    observed_exits
                        .entry((map, entry))
                        .or_default()
                        .insert(i32::from(*next));
                }
                break;
            }
        }
        let states = director.entry_states();
        let not_completed: Vec<String> = states
            .iter()
            .filter(|(_, _, status, _)| *status != VmStatus::Completed)
            .map(|(entity, pc, status, wait)| format!("{}@{pc}:{status:?}:{wait:?}", entity.get()))
            .collect();
        let scenario_counter = scenario_counter(director.world());
        println!(
            "{}",
            serde_json::json!({
                "map": map,
                "archive": archive,
                "decode_errors": decode_errors,
                "ticks": ticks_run,
                "operations": operations,
                "loops_started": scheduler.loops_started,
                "map_memory": nonzero(director.world().map_memory.bytes()),
                "global_memory": nonzero(director.world().global_memory.bytes()),
                "confirms": confirms,
                "field_text": text_status,
                "navigation": navigation_status,
                "mes_signal": director.world().mes_signal,
                "choices_made": choices_made,
                "battles_skipped": battles,
                "battles_fought": fights,
                "region_triggers": region_triggers,
                "confirm_presses": presses,
                "vms": states.len(),
                "spawned_pending": scheduler.spawned_pending(),
                "shared_scripts_run": scheduler.shared_scripts_run,
                "unspawned_requests": scheduler.unspawned,
                "encounters": encounters,
                "not_completed": not_completed,
                "error": error,
                "next_map": transition,
                "world_map": world_map,
                "party": director.world().party,
                "exits": exits,
                "route_target": route_target,
                "regions_defined": director.world().regions.keys().map(|region| region.get()).collect::<Vec<_>>(),
                "user_control": director.world().user_control,
                "control_changes": control_changes,
                "talks": talks,
                "texts_shown": texts_shown
                    .iter()
                    .map(|(tick, text)| {
                        let line: String = plain_texts
                            .get(text)
                            .map(|plain| plain.chars().take(90).collect())
                            .unwrap_or_default();
                        serde_json::json!([tick, text, line])
                    })
                    .collect::<Vec<_>>(),
                "control_actor": director.world().control_actor.map(|actor| actor.get()),
                "control_state": director
                    .world()
                    .control_actor
                    .and_then(|actor| director.world().actors.get(&actor))
                    .map(|state| (state.position, state.animation_frame, state.animation_frames)),
                "scenario_counter": scenario_counter,
            })
        );
        if let (Some(index), Some(next)) = (route_index, transition)
            && route[index].1 == next
        {
            route_used[index] = true;
        }
        world = director.world().clone();
        if transition.is_none()
            && let Some(world_map) = world_map
        {
            let scenario_now = scenario_counter;
            let index = world_exits.iter().enumerate().position(|(index, entry)| {
                entry.0 == world_map && !world_exits_used[index] && scenario_now >= entry.2
            });
            if let Some(index) = index {
                world_exits_used[index] = true;
                let location = world_exits[index].1;
                let outcome = match db.get(&world_map) {
                    Some(name) => tantalus_tools::field_session::world_exit(
                        bundle, name, location, &mut world,
                    ),
                    None => Err(format!("world map {world_map} not in EventDB")),
                };
                println!(
                    "{}",
                    serde_json::json!({
                        "world_map": world_map,
                        "location": location,
                        "next_map": outcome.as_ref().ok().copied().flatten(),
                        "error": outcome.as_ref().err(),
                    })
                );
                transition = outcome.ok().flatten();
            }
        }
        match transition {
            Some(next) => map = next,
            None => return Ok(()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tantalus_tools::field_data::parse_event_db;

    #[test]
    fn event_db_rows_parse_and_other_lines_are_ignored() {
        let source = "public static Dictionary<Int32, String> EventDB = new()\n    {\n        {70,    \"EVT_ALEX1_TS_OPENING\"},\n        {50, \"EVT_ALEX1_TS_CARGO_0\"},\n    };\n";
        let table = parse_event_db(source);
        assert_eq!(table.len(), 2);
        assert_eq!(table[&70], "EVT_ALEX1_TS_OPENING");
        assert_eq!(table[&50], "EVT_ALEX1_TS_CARGO_0");
    }
}
