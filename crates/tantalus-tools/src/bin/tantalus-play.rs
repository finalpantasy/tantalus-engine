//! Headless play: runs [`tantalus_tools::play::PlayGame`] with scripted pad
//! input and prints each event as a JSON line, so the client's `play`
//! window has a check that needs no screen.
//!
//! `tantalus-play --assets <StreamingAssets> --game-data <FF9_Data>
//! --memoria <root> --atan <ratan_tbl.bin> [--battles <tsv>] [--start 70]
//! [--ticks N] [--auto-confirm N] [--hold FROM-TO:MASK,...]
//! [--world-exits LOCATION,...]`

use std::path::PathBuf;
use std::process::ExitCode;

use tantalus_tools::play::{PlayData, PlayGame, PlayPaths, pad};

const USAGE: &str = "usage: tantalus-play --assets <StreamingAssets> --game-data <FF9_Data> --memoria <root> --atan <ratan_tbl.bin> [--battles <tsv>] [--start 70] [--ticks N] [--auto-confirm N] [--hold FROM-TO:MASK,...] [--world-exits LOCATION,...]";

fn main() -> ExitCode {
    // Opt-in Memoria `KeepRestTimeInBattle` (default off, vanilla).
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let flag = args.iter().any(|a| a == "--keep-rest-time");
    args.retain(|a| a != "--keep-rest-time");
    let env_on = std::env::var("TANTALUS_KEEP_REST_TIME").is_ok_and(|v| v == "1");
    tantalus_legacy::battle::machine::set_keep_rest_time_default(flag || env_on);
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tantalus-play: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut values = std::collections::BTreeMap::new();
    let mut iter = args.into_iter();
    while let Some(flag) = iter.next() {
        let value = iter
            .next()
            .ok_or_else(|| format!("{flag} needs a value\n{USAGE}"))?;
        values.insert(flag, value);
    }
    let path = |flag: &str| {
        values
            .get(flag)
            .map(PathBuf::from)
            .ok_or_else(|| format!("missing {flag}\n{USAGE}"))
    };
    let number = |flag: &str, default: u64| -> Result<u64, String> {
        values.get(flag).map_or(Ok(default), |value| {
            value.parse().map_err(|_| format!("bad {flag}"))
        })
    };
    let paths = PlayPaths {
        streaming_assets: path("--assets")?,
        game_data: path("--game-data")?,
        memoria: path("--memoria")?,
        atan_table: path("--atan")?,
        battles: values.get("--battles").map(PathBuf::from),
        language: values
            .get("--language")
            .cloned()
            .unwrap_or_else(|| "us".into()),
    };
    let start = number("--start", 70)? as i32;
    let ticks = number("--ticks", 3000)?;
    let auto_confirm = number("--auto-confirm", 0)?;
    let mut holds: Vec<(u64, u64, i32)> = Vec::new();
    if let Some(list) = values.get("--hold") {
        for part in list.split(',') {
            let parsed = part.split_once(':').and_then(|(range, mask)| {
                let (from, to) = range.split_once('-')?;
                let mask = mask.trim_start_matches("0x");
                Some((
                    from.parse().ok()?,
                    to.parse().ok()?,
                    i32::from_str_radix(mask, 16).ok()?,
                ))
            });
            holds.push(
                parsed.ok_or_else(|| format!("bad --hold entry `{part}`, want FROM-TO:HEXMASK"))?,
            );
        }
    }
    let data = PlayData::load(&paths)?;
    let (mut game, events) = PlayGame::new(data, start)?;
    if let Some(list) = values.get("--world-exits") {
        game.world_exits = list
            .split(',')
            .map(|value| {
                value
                    .parse()
                    .map_err(|_| format!("bad world exit `{value}`"))
            })
            .collect::<Result<_, _>>()?;
    }
    for event in events {
        println!(
            "{}",
            serde_json::to_string(&event).map_err(|e| e.to_string())?
        );
    }
    let mut open_for = 0u64;
    let mut last_confirm = false;
    for tick in 0..ticks {
        let mut held: i32 = holds
            .iter()
            .filter(|(from, to, _)| (*from..*to).contains(&tick))
            .fold(0, |mask, (_, _, bits)| mask | bits);
        open_for = if game.open_text().is_empty() {
            0
        } else {
            open_for + 1
        };
        // Release between presses so each one is a new press.
        if auto_confirm > 0 && open_for >= auto_confirm && !last_confirm {
            held |= pad::CONFIRM;
            open_for = 0;
        }
        last_confirm = held & pad::CONFIRM != 0;
        for event in game.tick(held)? {
            let mut line = serde_json::to_value(&event).map_err(|e| e.to_string())?;
            line["tick"] = tick.into();
            println!("{line}");
        }
        if game.stopped.is_some() {
            break;
        }
    }
    let world = game.world();
    let player = world.player().and_then(|actor| world.actors.get(&actor));
    let scenario = {
        let bytes = world.global_memory.bytes();
        i32::from(bytes[0]) | (i32::from(bytes[1] as i8) << 8)
    };
    println!(
        "{}",
        serde_json::json!({
            "summary": true,
            "ticks": game.ticks,
            "map": game.map(),
            "field": game.field_name(),
            "scenario": scenario,
            "user_control": world.user_control,
            "player": player.map(|actor| actor.position),
            "stopped": game.stopped,
            "regions": world.regions.iter().map(|(entry, region)| {
                let n = region.vertices.len().max(1) as i32;
                let (x, z) = region.vertices.iter().fold((0i32, 0i32), |(x, z), v| (x + i32::from(v[0]), z + i32::from(v[1])));
                (entry.get(), x / n, z / n, region.vertices.clone())
            }).collect::<Vec<_>>(),
        })
    );
    Ok(())
}
