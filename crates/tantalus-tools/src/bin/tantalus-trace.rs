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
       tantalus-trace chain --bundle <path> --event-db <FF9DBAll.Events.cs> --atan-table <path> [--start N] [--max-fields N] [--ticks N] [--confirm-after N] [--explore] [--walk-demo] [--character-data <Data/Characters dir>] [--item-table <Assembly-CSharp.dll>] [--game-data <FF9_Data dir> --mes-map <EventEngineUtils.cs> [--language us]] [--choices MAP:TEXT=N,...] [--route MAP>DEST,...] [--skip-talk MAP:ENTRY,...] [--hold MAP:MASK,...] [--waypoints MAP@X:Z/X:Z,...]";

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
    let mut character_data: Option<PathBuf> = None;
    let mut game_data: Option<PathBuf> = None;
    let mut mes_map: Option<PathBuf> = None;
    let mut item_table: Option<PathBuf> = None;
    let mut language = String::from("us");
    let mut choices: BTreeMap<(i32, u16), i32> = BTreeMap::new();
    let mut route: Vec<(i32, i32)> = Vec::new();
    let mut skip_talk: BTreeSet<(i32, usize)> = BTreeSet::new();
    let mut hold: BTreeMap<i32, i32> = BTreeMap::new();
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
            "--game-data" => game_data = Some(PathBuf::from(value()?)),
            "--mes-map" => mes_map = Some(PathBuf::from(value()?)),
            "--language" => language = value()?,
            "--choices" => {
                for part in value()?.split(',') {
                    let parsed = part.trim().split_once(':').and_then(|(map, rest)| {
                        let (text, answer) = rest.split_once('=')?;
                        Some((map.parse().ok()?, text.parse().ok()?, answer.parse().ok()?))
                    });
                    let (map, text, answer) = parsed
                        .ok_or_else(|| format!("bad --choices entry `{part}`, want MAP:TEXT=N"))?;
                    choices.insert((map, text), answer);
                }
            }
            "--route" => {
                for part in value()?.split(',') {
                    let parsed = part
                        .trim()
                        .split_once('>')
                        .and_then(|(map, dest)| Some((map.parse().ok()?, dest.parse().ok()?)));
                    route.push(
                        parsed
                            .ok_or_else(|| format!("bad --route entry `{part}`, want MAP>DEST"))?,
                    );
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
            let mut growth_tables: Option<(
                BTreeMap<i32, tantalus_game::new_game::BaseStats>,
                Vec<tantalus_game::new_game::LevelRow>,
            )> = None;
            let stats = match character_data {
                Some(dir) => {
                    let read = |name: &str| {
                        let path = dir.join(name);
                        std::fs::read_to_string(&path)
                            .map_err(|error| format!("cannot read {}: {error}", path.display()))
                    };
                    let base = tantalus_game::new_game::parse_base_stats(&read("BaseStats.csv")?)?;
                    let levels = tantalus_game::new_game::parse_leveling(&read("Leveling.csv")?)?;
                    growth_tables = Some((base.clone(), levels.clone()));
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
                    stats,
                    text,
                    choices,
                    game_data,
                    growth_tables,
                    item_ids,
                    route,
                    skip_talk,
                    hold,
                    backgrounds,
                    waypoints,
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
fn load_event(bundle_path: &Path, name: &str) -> Result<EventBinary, String> {
    let script = find_text_asset(bundle_path, name)?
        .ok_or_else(|| format!("event asset `{name}` was not found"))?;
    EventBinary::parse(&script).map_err(|error| error.to_string())
}

fn find_text_asset(bundle_path: &Path, name: &str) -> Result<Option<Vec<u8>>, String> {
    let bundle = tantalus_assets::pc::unity_raw::UnityRawBundle::open(bundle_path)
        .map_err(|error| error.to_string())?;
    let bytes = bundle
        .read_entry(0, 300 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    let serialized = tantalus_assets::pc::serialized::SerializedFile::parse(
        &bytes,
        &bundle_path.to_string_lossy(),
    )
    .map_err(|error| error.to_string())?;
    for (index, object) in serialized.report().objects.iter().enumerate() {
        if object.class_id != 49 {
            continue;
        }
        let asset = tantalus_assets::pc::text_asset::TextAsset::parse(
            serialized
                .object_data(index)
                .map_err(|error| error.to_string())?,
            name,
        )
        .map_err(|error| error.to_string())?;
        if asset.name == name {
            return Ok(Some(asset.script));
        }
    }
    Ok(None)
}

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

/// Parses `{id, "EVT_NAME"},` rows of Memoria's `FF9DBAll.EventDB`.
fn parse_event_db(source: &str) -> BTreeMap<i32, String> {
    let mut table = BTreeMap::new();
    for line in source.lines() {
        let Some(body) = line.trim().strip_prefix('{') else {
            continue;
        };
        let Some((id, rest)) = body.split_once(',') else {
            continue;
        };
        let Ok(id) = id.trim().parse::<i32>() else {
            continue;
        };
        let mut quoted = rest.split('"');
        let (Some(_), Some(name)) = (quoted.next(), quoted.next()) else {
            continue;
        };
        table.insert(id, name.to_owned());
    }
    table
}

type Pool = Vec<Vec<(u16, EventProgram)>>;

/// Registers one function of `entry` as a scene VM (key = entry * 256 +
/// function ordinal) acting on the entry's object. Returns `None` when the tag is absent or the VM is
/// already registered.
fn register(
    director: &mut tantalus_game::FieldDirector,
    pool: &Pool,
    entry: usize,
    tag: Option<u16>,
) -> Result<Option<EntityId>, String> {
    let functions = &pool[entry];
    let position = match tag {
        Some(tag) => functions.iter().position(|(t, _)| *t == tag),
        None => (!functions.is_empty()).then_some(0),
    };
    let Some(position) = position else {
        return Ok(None);
    };
    let entity = EntityId::new(entry as u64 * 256 + position as u64);
    if director
        .entry_states()
        .iter()
        .any(|(existing, _, _, _)| *existing == entity)
    {
        return Ok(None);
    }
    // Every function of an entry acts on the entry's object (one `Obj` in
    // the reference), registered per field as `EntityId(entry)` with uid
    // `entry`.
    director
        .add_object_function(
            entity,
            EntityId::new(entry as u64),
            functions[position].1.clone(),
        )
        .map_err(|error| error.to_string())?;
    Ok(Some(entity))
}

fn decode_pool(binary: &EventBinary) -> Result<(Pool, usize), String> {
    let mut pool: Pool = Vec::new();
    let mut decode_errors = 0usize;
    for entry in &binary.entries {
        let decoded = binary
            .decode_entry_with_diagnostics(entry.index)
            .map_err(|error| error.to_string())?;
        decode_errors += decoded.errors.len();
        pool.push(
            decoded
                .functions
                .into_iter()
                .filter(|function| !function.program.instructions.is_empty())
                .map(|function| (function.tag, function.program))
                .collect(),
        );
    }
    Ok((pool, decode_errors))
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
const CONFIRM_PERIOD: u64 = 15;
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
/// VM key base for shared scripts (`UID_OFFSET_SEQ`): above every
/// `entry * 256 + function` key.
const SHARED_SCRIPT_KEY: u64 = 0x1_0000;

/// Entity `register` uses for an entry's function with `tag`.
fn function_entity(pool: &Pool, entry: usize, tag: u16) -> Option<EntityId> {
    let position = pool[entry].iter().position(|(t, _)| *t == tag)?;
    Some(EntityId::new(entry as u64 * 256 + position as u64))
}

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

fn centroid(vertices: &[[i16; 2]]) -> [f32; 2] {
    let n = vertices.len().max(1) as f32;
    let (x, z) = vertices.iter().fold((0.0, 0.0), |(x, z), [vx, vz]| {
        (x + f32::from(*vx), z + f32::from(*vz))
    });
    [x / n, z / n]
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
    stats: BTreeMap<i32, tantalus_game::field::CharacterStats>,
    text: Option<FieldText>,
    /// Scripted answers for choice windows by (field map, text id); 0 when
    /// absent, the reference's default cursor position.
    choices: BTreeMap<(i32, u16), i32>,
    /// `FF9_Data` directory for the FMV movie inventory (`StreamingAssets/ma`).
    game_data: Option<PathBuf>,
    /// Level tables for experience growth (base stats and leveling rows).
    growth_tables: Option<(
        BTreeMap<i32, tantalus_game::new_game::BaseStats>,
        Vec<tantalus_game::new_game::LevelRow>,
    )>,
    /// Vanilla item-table legacy ids for the reward-item identity mapping.
    item_ids: Option<Vec<i32>>,
    /// Free-roam route: while MAP's first unused entry is DEST, the explorer
    /// only enters exit regions that lead to DEST; the entry is used up when
    /// the field leaves for DEST. Regions whose tag 2 holds no
    /// constant `ChangeField` stay allowed.
    route: Vec<(i32, i32)>,
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

/// Per-field text facts the scripted player needs.
#[derive(Default)]
struct FieldTextInfo {
    signals: BTreeMap<u16, Vec<i32>>,
    /// Text ids that contain a `[CHOO]` choice list.
    choices: BTreeSet<u16>,
    /// Literal text of each entry (tags dropped), for the trace record.
    plain: BTreeMap<u16, String>,
}

/// Field dialog text, for the `[SIGL=n]` signals windows raise.
///
/// Memoria loads `EmbeddedAsset/Text/<lang>/Field/<zone>.mes`, where the zone
/// is `EventEngineUtils.eventIDToMESID[fldMapNo]` (`HonoluluFieldMain`,
/// `FF9TextTool.UpdateFieldText`). The path resolves through the Unity
/// `ResourceManager` container in `mainData`. The map table is read at run
/// time from the pinned Memoria source, not copied here.
struct FieldText {
    data_dir: PathBuf,
    language: String,
    zones: BTreeMap<i32, i32>,
    /// Lower-case resource path -> (file name, path id).
    container: BTreeMap<String, (String, i64)>,
}

impl FieldText {
    fn open(data_dir: &Path, map_source: &str, language: String) -> Result<Self, String> {
        use tantalus_assets::pc::resource_container::{
            RESOURCE_MANAGER_CLASS_ID, parse_resource_container,
        };
        use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
        let main = data_dir.join("mainData");
        let mut file = StandaloneSerialized::open(&main).map_err(|error| error.to_string())?;
        let manager = file
            .objects()
            .iter()
            .find(|object| object.class_id == RESOURCE_MANAGER_CLASS_ID)
            .cloned()
            .ok_or("mainData has no ResourceManager")?;
        let bytes = file
            .read_object(&manager)
            .map_err(|error| error.to_string())?;
        let externals = file.inventory().external_paths.clone();
        let mut container = BTreeMap::new();
        for entry in parse_resource_container(&bytes, &main.display().to_string())
            .map_err(|error| error.to_string())?
        {
            let file_name = match usize::try_from(entry.file_id) {
                Ok(0) => "mainData".to_owned(),
                Ok(index) => match externals.get(index - 1) {
                    Some(path) => path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned(),
                    None => continue,
                },
                Err(_) => continue,
            };
            container.insert(entry.path.to_lowercase(), (file_name, entry.path_id));
        }
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            language,
            zones: parse_mes_map(map_source),
            container,
        })
    }

    /// `[SIGL]` values and choice texts for one field map, or why none were
    /// loaded.
    fn info(&self, map: i32) -> Result<FieldTextInfo, String> {
        use tantalus_assets::pc::mes::{MesToken, parse_mes_bytes};
        use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
        let zone = self
            .zones
            .get(&map)
            .ok_or_else(|| format!("map {map} has no eventIDToMESID entry"))?;
        let key = format!("embeddedasset/text/{}/field/{zone}", self.language);
        let (file_name, path_id) = self
            .container
            .get(&key)
            .or_else(|| self.container.get(&format!("{key}.mes")))
            .ok_or_else(|| format!("resource `{key}` not in the container"))?;
        let path = self.data_dir.join(file_name);
        let mut file = StandaloneSerialized::open(&path).map_err(|error| error.to_string())?;
        let object = file
            .objects()
            .iter()
            .find(|object| object.path_id == *path_id)
            .cloned()
            .ok_or_else(|| format!("{file_name} has no object {path_id}"))?;
        let asset = file
            .read_text_asset(&object)
            .map_err(|error| error.to_string())?;
        let mes = parse_mes_bytes(&asset.script, &key).map_err(|error| error.to_string())?;
        let mut info = FieldTextInfo::default();
        for entry in &mes.entries {
            let Ok(id) = u16::try_from(entry.id) else {
                continue;
            };
            let plain: String = entry
                .tokens
                .iter()
                .filter_map(|token| match token {
                    MesToken::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            info.plain.insert(id, plain);
            if entry
                .tokens
                .iter()
                .any(|token| matches!(token, MesToken::Tag(tag) if tag.code == "Choice"))
            {
                info.choices.insert(id);
            }
            let values: Vec<i32> = entry
                .tokens
                .iter()
                .filter_map(|token| match token {
                    MesToken::Tag(tag) if tag.code == "Signal" => tag.params.first()?.parse().ok(),
                    _ => None,
                })
                .collect();
            if !values.is_empty() {
                info.signals.insert(id, values);
            }
        }
        Ok(info)
    }
}

/// `EventEngineUtils.eventIDToMESID` entries (`{map, zone},`), read from the
/// Memoria source after the dictionary's declaration.
fn parse_mes_map(source: &str) -> BTreeMap<i32, i32> {
    let Some(start) = source.find("eventIDToMESID") else {
        return BTreeMap::new();
    };
    let mut map = BTreeMap::new();
    for line in source[start..].lines().skip(1) {
        let line = line.trim();
        if line.starts_with("};") {
            break;
        }
        let Some(body) = line.strip_prefix('{') else {
            continue;
        };
        let mut parts = body.trim_end_matches(',').trim_end_matches('}').split(',');
        if let (Some(Ok(map_no)), Some(Ok(zone))) = (
            parts.next().map(|part| part.trim().parse()),
            parts.next().map(|part| part.trim().parse()),
        ) {
            map.insert(map_no, zone);
        }
    }
    map
}

/// `EventEngineUtils.eventIDToFBGID` entries (`{map, "FBG_..."},`), read
/// from the Memoria source after the dictionary's declaration.
fn parse_background_map(source: &str) -> BTreeMap<i32, String> {
    let Some(start) = source.find("eventIDToFBGID") else {
        return BTreeMap::new();
    };
    let mut map = BTreeMap::new();
    for line in source[start..].lines().skip(1) {
        let line = line.trim();
        if line.starts_with("};") {
            break;
        }
        let Some(body) = line.strip_prefix('{') else {
            continue;
        };
        let Some((id, rest)) = body.split_once(',') else {
            continue;
        };
        let Ok(id) = id.trim().parse::<i32>() else {
            continue;
        };
        if let Some(name) = rest.split('"').nth(1) {
            map.insert(id, name.to_owned());
        }
    }
    map
}

/// Finds the field bundle that registers `name` and loads its walkmesh.
fn load_navigation(
    bundles: &[PathBuf],
    name: &str,
) -> Result<tantalus_contracts::FieldNavigationMesh, String> {
    for bundle in bundles {
        if !bundle.exists() {
            continue;
        }
        match tantalus_assets::pc::bundled_field::load_field_navigation(bundle, name) {
            Ok(Some(mesh)) => return Ok(mesh),
            Ok(None) => {}
            Err(error) => return Err(format!("{}: {error}", bundle.display())),
        }
    }
    Err(format!("no field bundle registers `{name}`"))
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
        stats,
        text,
        choices,
        game_data,
        growth_tables,
        item_ids,
        route,
        skip_talk,
        hold,
        backgrounds,
        waypoints,
    } = options;
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
    for _ in 0..max_fields {
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
        let exits: BTreeMap<usize, i32> = (0..pool.len())
            .filter_map(|entry| Some((entry, exit_destination(&pool[entry])?)))
            .collect();
        // The first unused route entry for this map; it is used up when the
        // field leaves through its destination.
        let route_index = route
            .iter()
            .enumerate()
            .position(|(index, (from, _))| *from == map && !route_used[index]);
        let route_target = route_index.map(|index| route[index].1);
        world.field_map_no = map;
        let held = hold.get(&map).copied().unwrap_or(0);
        // EventEngine.InitEvents zeroes mapvar for every field (EventEngine.cs:556).
        world.map_memory.clear();
        world.clear_deleted_objects();
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
        world.take_field_transitions();
        // Field load closes every window.
        world.windows.clear();
        world.window_texts.clear();
        let mut director = tantalus_game::FieldDirector::new(world.clone());
        director.world_mut().set_input(held);
        for entry in 0..pool.len() {
            let base = EntityId::new(entry as u64);
            director.world_mut().add_actor(base, [0.0; 3]);
            director.world_mut().map_uid(entry as i32, base);
        }
        for slot in 251u64..256 {
            let entity = EntityId::new(slot);
            director.world_mut().add_actor(entity, [0.0; 3]);
            director.world_mut().map_uid(slot as i32, entity);
        }
        // Objects still in their init function (tag 0). When it returns,
        // EventEngine.Return (EventEngine.cs:1178) moves the object to
        // stateRunning and starts its loop function (tag 1) at level 7.
        let mut initializing: Vec<(EntityId, usize)> = Vec::new();
        let mut shared_running: BTreeMap<EntityId, EntityId> = BTreeMap::new();
        let mut shared_scripts_run = 0usize;
        for entry in 0..pool.len() {
            if let Some(entity) = register(&mut director, &pool, entry, Some(0))? {
                initializing.push((entity, entry));
            }
        }
        let mut loops_started = 0usize;
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
        let mut spawned: Vec<(EntityId, u64)> = Vec::new();
        let mut unspawned = 0usize;
        let mut encounters = 0usize;
        let mut battles: Vec<i32> = Vec::new();
        // (tick, user control, Map byte 24) whenever user control changes.
        let mut control_changes: Vec<(u64, bool, u8)> = Vec::new();
        // Each text a window shows, in order, as (tick, text id).
        let mut texts_shown: Vec<(u64, u16)> = Vec::new();
        let mut last_texts: BTreeMap<u8, u16> = BTreeMap::new();
        let mut last_control = director.world().user_control;
        while ticks_run < ticks {
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
                    encounters += step.encounters.len();
                    // No battle engine yet: each scripted battle is skipped
                    // and the field resumes on the next tick, as after a win.
                    battles.extend(step.encounters.iter().map(|request| request.battle));
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
                        for trigger in &step.talk_triggers {
                            let entry = trigger.get() as usize;
                            if entry >= pool.len() || talked.contains(&entry) {
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
                    // STARTSEQ: a new shared script replaces the object's
                    // previous one (`DisposeObj` of uid + UID_OFFSET_SEQ).
                    for (object, entry) in &step.shared_scripts {
                        let Ok(entry) = usize::try_from(*entry) else {
                            continue;
                        };
                        let Some((_, program)) = pool.get(entry).and_then(|f| f.first()) else {
                            continue;
                        };
                        let key = EntityId::new(SHARED_SCRIPT_KEY + object.get());
                        director.remove_entry(key);
                        director
                            .add_shared_script(key, *object, program.clone())
                            .map_err(|error| error.to_string())?;
                        shared_running.insert(key, *object);
                        shared_scripts_run += 1;
                    }
                    for request in &step.requests {
                        let target = request.target.get() as usize;
                        if target >= pool.len() {
                            unspawned += 1;
                            continue;
                        }
                        let tag = u16::from(request.tag);
                        let tag = pool[target].iter().any(|(t, _)| *t == tag).then_some(tag);
                        // A function that already ran to its end runs again
                        // from its start (each request is a fresh call).
                        if let Some(entity) =
                            tag.and_then(|tag| function_entity(&pool, target, tag))
                            && director.entry_states().iter().any(|(e, _, status, _)| {
                                *e == entity && *status == VmStatus::Completed
                            })
                        {
                            director.remove_entry(entity);
                        }
                        match register(&mut director, &pool, target, tag)? {
                            Some(entity) => spawned.push((entity, target as u64)),
                            None => {
                                // Nothing will run, so nothing would ever
                                // clear the pend: release a waiting caller.
                                unspawned += 1;
                                if !spawned.iter().any(|(_, base)| *base == target as u64) {
                                    director
                                        .world_mut()
                                        .complete_request(EntityId::new(target as u64));
                                }
                            }
                        }
                    }
                }
                Err(failure) => {
                    error = Some(failure.to_string());
                    break;
                }
            }
            let states = director.entry_states();
            shared_running.retain(|key, object| {
                let complete = states
                    .iter()
                    .any(|(e, _, status, _)| e == key && *status == VmStatus::Completed);
                if complete {
                    director.world_mut().finish_shared_script(*object);
                }
                !complete
            });
            let mut done = Vec::new();
            spawned.retain(|(entity, base)| {
                let complete = states
                    .iter()
                    .any(|(e, _, status, _)| e == entity && *status == VmStatus::Completed);
                if complete {
                    done.push(*base);
                }
                !complete
            });
            for base in done {
                director.world_mut().complete_request(EntityId::new(base));
            }
            let mut ready = Vec::new();
            initializing.retain(|(entity, entry)| {
                let complete = states
                    .iter()
                    .any(|(e, _, status, _)| e == entity && *status == VmStatus::Completed);
                if complete {
                    ready.push(*entry);
                }
                !complete
            });
            for entry in ready {
                if pool[entry].iter().any(|(tag, _)| *tag == 1)
                    && register(&mut director, &pool, entry, Some(1))?.is_some()
                {
                    loops_started += 1;
                }
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
                        if let Some(text) = shown.filter(|text| choice_texts.contains(text)) {
                            let answer = choices.get(&(map, text)).copied().unwrap_or(0);
                            director.world_mut().dialog_choice = answer;
                            choices_made.push((text, answer));
                        }
                        director.world_mut().close_window(window);
                        open_since.remove(&window);
                        confirms += 1;
                    }
                }
            }
            if explore {
                if pressed {
                    director.world_mut().set_input(held);
                    pressed = false;
                }
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
                    let has_tag2 = |region: &EntityId| {
                        let entry = region.get() as usize;
                        let routed = match (route_target, exits.get(&entry)) {
                            (Some(target), Some(dest)) => *dest == target,
                            _ => true,
                        };
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
                            .map(|(_, region)| centroid(&region.vertices))
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
                    match hit {
                        Some(entry) => {
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
                            if register(&mut director, &pool, entry, Some(2))?.is_some() {
                                region_triggers += 1;
                            }
                            let dwell = dwell_ticks.entry(entry).or_insert(0);
                            *dwell += 1;
                            if *dwell >= REGION_DWELL {
                                triggered.insert(entry);
                            }
                        }
                        None => {
                            if let (Some([x, z]), Some(state)) =
                                (next, director.world_mut().actors.get_mut(&actor))
                            {
                                state.position = [x, state.position[1], z];
                                state.movement = None;
                            }
                            // Give up on an object that never answers (out of
                            // range, hidden, or not placed yet).
                            if walking {
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
            let transitions = director.world_mut().take_field_transitions();
            if let Some(next) = transitions.first() {
                transition = Some(i32::from(*next));
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
                "loops_started": loops_started,
                "map_memory": nonzero(director.world().map_memory.bytes()),
                "global_memory": nonzero(director.world().global_memory.bytes()),
                "confirms": confirms,
                "field_text": text_status,
                "navigation": navigation_status,
                "mes_signal": director.world().mes_signal,
                "choices_made": choices_made,
                "battles_skipped": battles,
                "region_triggers": region_triggers,
                "confirm_presses": presses,
                "vms": states.len(),
                "spawned_pending": spawned.len(),
                "shared_scripts_run": shared_scripts_run,
                "unspawned_requests": unspawned,
                "encounters": encounters,
                "not_completed": not_completed,
                "error": error,
                "next_map": transition,
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
        match transition {
            Some(next) => map = next,
            None => return Ok(()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_event_db;

    #[test]
    fn event_db_rows_parse_and_other_lines_are_ignored() {
        let source = "public static Dictionary<Int32, String> EventDB = new()\n    {\n        {70,    \"EVT_ALEX1_TS_OPENING\"},\n        {50, \"EVT_ALEX1_TS_CARGO_0\"},\n    };\n";
        let table = parse_event_db(source);
        assert_eq!(table.len(), 2);
        assert_eq!(table[&70], "EVT_ALEX1_TS_OPENING");
        assert_eq!(table[&50], "EVT_ALEX1_TS_CARGO_0");
    }
}
