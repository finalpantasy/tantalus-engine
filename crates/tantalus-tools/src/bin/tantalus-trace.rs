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
use tantalus_legacy::{EventBinary, EventProgram, VmStatus};

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
       tantalus-trace chain --bundle <path> --event-db <FF9DBAll.Events.cs> --atan-table <path> [--start N] [--max-fields N] [--ticks N] [--confirm-after N] [--explore] [--character-data <Data/Characters dir>] [--game-data <FF9_Data dir> --mes-map <EventEngineUtils.cs> [--language us]] [--choices MAP:TEXT=N,...]";

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
    let mut character_data: Option<PathBuf> = None;
    let mut game_data: Option<PathBuf> = None;
    let mut mes_map: Option<PathBuf> = None;
    let mut language = String::from("us");
    let mut choices: BTreeMap<(i32, u16), i32> = BTreeMap::new();
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
            "--explore" => explore = true,
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
            let stats = match character_data {
                Some(dir) => {
                    let read = |name: &str| {
                        let path = dir.join(name);
                        std::fs::read_to_string(&path)
                            .map_err(|error| format!("cannot read {}: {error}", path.display()))
                    };
                    let base = tantalus_game::new_game::parse_base_stats(&read("BaseStats.csv")?)?;
                    let levels = tantalus_game::new_game::parse_leveling(&read("Leveling.csv")?)?;
                    tantalus_game::new_game::new_game_character_stats(&base, &levels)
                }
                None => BTreeMap::new(),
            };
            let text = match (game_data, mes_map) {
                (Some(data), Some(map_path)) => {
                    let source = std::fs::read_to_string(&map_path)
                        .map_err(|error| format!("cannot read {}: {error}", map_path.display()))?;
                    Some(FieldText::open(&data, &source, language)?)
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
                    stats,
                    text,
                    choices,
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

/// Registers one function of `entry` as a scene VM (entity = entry * 256 +
/// function ordinal). Returns `None` when the tag is absent or the VM is
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
    director
        .add_entry_program(entity, functions[position].1.clone())
        .map_err(|error| error.to_string())?;
    director.world_mut().add_actor(entity, [0.0; 3]);
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

/// Entity `register` uses for an entry's function with `tag`.
fn function_entity(pool: &Pool, entry: usize, tag: u16) -> Option<EntityId> {
    let position = pool[entry].iter().position(|(t, _)| *t == tag)?;
    Some(EntityId::new(entry as u64 * 256 + position as u64))
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
    stats: BTreeMap<i32, tantalus_game::field::CharacterStats>,
    text: Option<FieldText>,
    /// Scripted answers for choice windows by (field map, text id); 0 when
    /// absent, the reference's default cursor position.
    choices: BTreeMap<(i32, u16), i32>,
}

/// Per-field text facts the scripted player needs.
#[derive(Default)]
struct FieldTextInfo {
    signals: BTreeMap<u16, Vec<i32>>,
    /// Text ids that contain a `[CHOO]` choice list.
    choices: BTreeSet<u16>,
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
        stats,
        text,
        choices,
    } = options;
    let mut world = tantalus_game::FieldWorld::new();
    // FF9Play_Init: every character has a player record from new game on.
    world.character_stats = stats;
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
        world.field_map_no = map;
        // EventEngine.InitEvents zeroes mapvar for every field (EventEngine.cs:556).
        world.map_memory.clear();
        let mut choice_texts = BTreeSet::new();
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
        let mut director = tantalus_game::FieldDirector::new(world.clone());
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
        let mut triggered: BTreeSet<usize> = BTreeSet::new();
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
        while ticks_run < ticks {
            match director.step() {
                Ok(step) => {
                    ticks_run += 1;
                    operations += step.operations;
                    encounters += step.encounters.len();
                    // No battle engine yet: each scripted battle is skipped
                    // and the field resumes on the next tick, as after a win.
                    battles.extend(step.encounters.iter().map(|request| request.battle));
                    for request in &step.requests {
                        let target = request.target.get() as usize;
                        if target >= pool.len() {
                            unspawned += 1;
                            continue;
                        }
                        let tag = u16::from(request.tag);
                        let tag = pool[target].iter().any(|(t, _)| *t == tag).then_some(tag);
                        match register(&mut director, &pool, target, tag)? {
                            Some(entity) => spawned.push((entity, target as u64)),
                            None => unspawned += 1,
                        }
                    }
                }
                Err(failure) => {
                    error = Some(failure.to_string());
                    break;
                }
            }
            let states = director.entry_states();
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
                    director.world_mut().set_input(0);
                    pressed = false;
                }
                let world = director.world();
                let controlled = world
                    .user_control
                    .then_some(world.control_actor)
                    .flatten()
                    .and_then(|actor| Some((actor, world.actors.get(&actor)?.position)));
                if let Some((actor, position)) = controlled {
                    let has_tag2 = |region: &EntityId| {
                        let entry = (region.get() / 256) as usize;
                        entry < pool.len() && pool[entry].iter().any(|(tag, _)| *tag == 2)
                    };
                    let hit = tantalus_game::field::region::regions_at(
                        &world.regions,
                        position[0],
                        position[2],
                    )
                    .into_iter()
                    .find(|region| {
                        has_tag2(region) && !triggered.contains(&((region.get() / 256) as usize))
                    })
                    .map(|region| (region.get() / 256) as usize);
                    let next = world
                        .regions
                        .iter()
                        .find(|(region, _)| {
                            has_tag2(region)
                                && !triggered.contains(&((region.get() / 256) as usize))
                        })
                        .map(|(_, region)| centroid(&region.vertices));
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
                        }
                    }
                    if ticks_run.is_multiple_of(CONFIRM_PERIOD) {
                        director.world_mut().set_input(CONFIRM);
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
                "mes_signal": director.world().mes_signal,
                "choices_made": choices_made,
                "battles_skipped": battles,
                "region_triggers": region_triggers,
                "confirm_presses": presses,
                "vms": states.len(),
                "spawned_pending": spawned.len(),
                "unspawned_requests": unspawned,
                "encounters": encounters,
                "not_completed": not_completed,
                "error": error,
                "next_map": transition,
            })
        );
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
