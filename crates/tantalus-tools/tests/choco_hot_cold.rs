//! Chocobo Hot and Cold in Chocobo's Forest (`EVT_CHOCO_CH_FST_0`, field 2950), end to end:
//! Mene's menu (a talk region, `EventCollision.CheckQuadInput`), the 60 gil fee, the how-to
//! page closed with Special (square), the clock (`TimerUI`), digging (Special, `KeyOn(0x80000)`)
//! with Choco's hints, and the end: the tally of points. Needs the game
//! install, so it returns early without one.

use std::path::PathBuf;
use tantalus_contracts::EntityId;
use tantalus_legacy::VariableType as V;
use tantalus_tools::play::{PlayData, PlayGame, PlayPaths, pad};

/// A save-like state: the intro is over (bit 1423), Mene's tutorials are seen, Choco is
/// ridden (byte 190) and has its second ability (byte 191), the story is late (10000).
const STATE: [(u16, V, i32); 13] = [
    (1423, V::Bit, 1),
    (810, V::Bit, 1),
    (809, V::Bit, 1),
    (190, V::Byte, 1),
    (191, V::Byte, 2),
    (0, V::UInt16, 10000),
    (1046, V::Bit, 1),
    (1047, V::Bit, 1),
    (1159, V::Bit, 1),
    (1158, V::Bit, 1),
    (1156, V::Bit, 1),
    (1045, V::Bit, 1),
    (1087, V::Bit, 1),
];

fn texts(game: &PlayGame) -> Vec<String> {
    game.open_text().into_iter().map(|(_, text)| text).collect()
}

#[test]
fn hot_and_cold_runs_from_mene_to_the_chocograph() {
    let root = PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX");
    let memoria = PathBuf::from("E:/FFIX Recomp/.reference/Memoria");
    let atan = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../local/probe/ratan_tbl.bin");
    if !root.is_dir() || !memoria.is_dir() || !atan.is_file() {
        return;
    }
    let paths = PlayPaths {
        streaming_assets: root.join("StreamingAssets"),
        game_data: root.join("x64/FF9_Data"),
        memoria,
        atan_table: atan,
        battles: None,
        language: "us".to_owned(),
    };
    let data = PlayData::load(&paths).expect("play data");
    let (mut game, _) = PlayGame::new_with_globals(data, 2950, &STATE).expect("game");
    game.set_gil(5000);

    let mut open_for = 0u32;
    let mut last_pressed = false;
    let mut seen: Vec<String> = Vec::new();
    let mut timer_started = None;
    let mut dig = 0usize;
    let mut menu_seen = false;
    let mut hits = Vec::new();
    for tick in 0..4400u64 {
        let mut held = 0;
        // Choco walks in, then stands in Mene's talk region and presses confirm.
        if tick == 370 {
            game.place_player([700.0, -268.0, -4450.0]);
        }
        if (372..376).contains(&tick) {
            held |= pad::RIGHT;
        }
        if (300..470).contains(&tick) && tick % 40 == 0 && game.open_text().is_empty() {
            held |= pad::CONFIRM;
        }
        // Digging: Special on each treasure's spot in turn, and Special to dig it up.
        if timer_started.is_some() {
            let world = game.world();
            let digging = world.user_control
                && texts(&game)
                    .iter()
                    .all(|t| !t.contains("Time") && !t.starts_with("Found") && !t.contains("Kweh"));
            if tick % 15 == 0 && digging {
                if dig.is_multiple_of(2) {
                    let mut spots: Vec<u64> = world
                        .actors
                        .keys()
                        .map(|id| id.get())
                        .filter(|id| *id >= 65544 && id % 65536 == 8)
                        .collect();
                    spots.sort();
                    let at = world.actors[&EntityId::new(spots[(dig / 2) % spots.len()])].position;
                    game.place_player(at);
                }
                dig += 1;
                held |= pad::SQUARE;
            } else if tick % 6 == 3 {
                // Dig, dig, dig: mash Special while the treasure comes up.
                held |= pad::SQUARE;
            }
        }
        open_for = if game.open_text().is_empty() {
            0
        } else {
            open_for + 1
        };
        if tick >= 300 && open_for >= 30 && !last_pressed && held == 0 {
            held |= if texts(&game)
                .iter()
                .any(|t| t.contains("How to play") || t.contains("Cries"))
            {
                pad::SQUARE
            } else {
                pad::CONFIRM
            };
            open_for = 0;
        }
        last_pressed = held != 0;
        game.tick(held).expect("tick");
        for text in texts(&game) {
            if !seen.contains(&text) {
                seen.push(text.clone());
            }
            if text.contains("Play Chocobo Hot") {
                menu_seen = true;
            }
            if text.contains("Kweh") && hits.len() < 3 {
                hits.push((tick, text));
            }
        }
        let world = game.world();
        if seen.iter().any(|text| text.contains("Found # point(s)"))
            && texts(&game)
                .iter()
                .any(|t| t.contains("Check current points"))
            && world.timer.time_int() == 0
        {
            break;
        }
        if timer_started.is_none() && world.timer.displayed && world.timer.running {
            timer_started = Some(tick);
            // `SetTimerTime(60 / Map.SignedByte[36] + 1)`: 61 seconds, shown and counting.
            assert_eq!(world.timer.time_int(), 61);
            assert_eq!(world.gil, 4940, "the 60 gil fee was paid");
        }
    }
    assert!(menu_seen, "Mene's menu opened from the talk region");
    let started = timer_started.expect("the clock started");
    assert!(started < 700, "started at {started}");
    assert!(!hits.is_empty(), "Choco answered a dig with a hint");
    assert!(hits[0].0 > started, "the hint came after the clock started");
    let any = |needle: &str| seen.iter().any(|text| text.contains(needle));
    assert!(any("How to play Chocobo Hot"), "the how-to page showed");
    assert!(any("Found "), "a treasure was dug up");
    assert!(
        !any("carry any more"),
        "the card pickup said it could not carry"
    );
    assert!(any("Found # point(s)"), "the end tallied the points");
    assert!(any("Check current points"), "Mene's menu came back");
    let world = game.world();
    assert_eq!(world.timer.time_int(), 0);
}
