//! The world map's moogle and its save: Square on foot calls the moogle (the world script,
//! `EVT_WORLD_WORLD00` entries 3, 2 and 6), whose Save offer opens the save screen, and a
//! world map save loads back at the same spot and in the same vehicle. Real-data checks;
//! they skip without the install and the audit save slots under `local/probe/play-audit/`.
use std::path::PathBuf;

use tantalus_tools::field_menu::SlotRow;
use tantalus_tools::play::{PlayData, PlayGame, PlayPaths, pad};

fn paths() -> Option<PlayPaths> {
    let root = PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX");
    let memoria = PathBuf::from("E:/FFIX Recomp/.reference/Memoria");
    let atan = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../local/probe/ratan_tbl.bin");
    if !root.is_dir() || !memoria.is_dir() || !atan.is_file() {
        return None;
    }
    Some(PlayPaths {
        streaming_assets: root.join("StreamingAssets"),
        game_data: root.join("x64/FF9_Data"),
        memoria,
        atan_table: atan,
        battles: None,
        language: "us".to_owned(),
    })
}

/// A game on the world map (audit slot 6, map 9000) on foot, saving to a fresh folder.
fn on_the_world_map(name: &str) -> Option<(PlayGame, PathBuf)> {
    let paths = paths()?;
    let save = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../local/probe/play-audit/saves/slot6.json");
    if !save.is_file() {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("tantalus-moogle-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::copy(&save, dir.join("slot1.json")).expect("copy save");
    let data = PlayData::load(&paths).expect("play data");
    let (mut game, _) = PlayGame::new(data, 70).expect("game");
    game.set_save_dir(dir.clone());
    let mut events = Vec::new();
    game.load_slot(1, &mut events).expect("load");
    for _ in 0..30 {
        game.tick(0).expect("tick");
    }
    Some((game, dir))
}

fn texts(game: &PlayGame) -> Vec<String> {
    game.open_text().into_iter().map(|(_, text)| text).collect()
}

/// Presses `bits` for one tick, then lets `rest` ticks pass.
fn press(game: &mut PlayGame, bits: i32, rest: usize) {
    game.tick(bits).expect("tick");
    for _ in 0..rest {
        game.tick(0).expect("tick");
    }
}

/// Ticks until a window shows `needle`; panics after `limit` ticks.
fn until_text(game: &mut PlayGame, needle: &str, limit: usize) {
    for _ in 0..limit {
        if texts(game).iter().any(|text| text.contains(needle)) {
            return;
        }
        game.tick(0).expect("tick");
    }
    panic!("no window said `{needle}`: {:?}", texts(game));
}

/// Walks the moogle's menus to the save screen.
fn open_the_save_screen(game: &mut PlayGame) {
    press(game, pad::SQUARE, 0);
    until_text(game, "Can I help you, kupo?", 200);
    // Save (the first row), then "Save, kupo?" Save (the first row).
    press(game, pad::CONFIRM, 1);
    until_text(game, "Save, kupo?", 100);
    press(game, pad::CONFIRM, 1);
    // "Let's save, kupo!" waits for a confirm too.
    for tick in 0..300 {
        if game.menu_frame().is_some() {
            return;
        }
        game.tick(if tick % 20 == 10 { pad::CONFIRM } else { 0 })
            .expect("tick");
    }
    panic!("the save screen did not open");
}

/// Square on foot is the moogle, not the yellow chocobo (`EVT_WORLD_WORLD00` entry 3:
/// `KeyOn(Special)` with the ride (gEventGlobal 190) at 0 and the map variables 37 and 41 off
/// sets map variable 30, entry 2 requests the moogle, entry 6 function 25 walks it in and asks
/// "Can I help you, kupo?" with Save, Tent and Cancel). On a chocobo Square gets off and calls
/// nothing; the moogle stays away.
#[test]
fn square_on_foot_calls_the_moogle_and_not_the_chocobo() {
    let Some((mut game, dir)) = on_the_world_map("call") else {
        return;
    };
    assert_eq!(game.world_play.as_ref().expect("walking").ride, 0);
    press(&mut game, pad::SQUARE, 0);
    until_text(&mut game, "Can I help you, kupo?", 200);
    assert_eq!(
        game.world_play.as_ref().expect("walking").ride,
        0,
        "no chocobo came"
    );
    let window = texts(&game).remove(0);
    assert!(
        window.contains("Save") && window.contains("Tent"),
        "{window}"
    );
    // Cancel (the third row): the window closes and the player walks again.
    press(&mut game, pad::DOWN, 1);
    press(&mut game, pad::DOWN, 1);
    press(&mut game, pad::CONFIRM, 150);
    assert!(
        !texts(&game).iter().any(|text| text.contains("Can I help")),
        "{:?}",
        texts(&game)
    );
    assert_eq!(game.world_play.as_ref().expect("walking").ride, 0);

    // His goodbye ("Don't call me if you don't need me, kupo!") takes a confirm.
    for tick in 0..200 {
        if texts(&game).is_empty() && tick > 20 {
            break;
        }
        game.tick(if tick % 20 == 10 { pad::CONFIRM } else { 0 })
            .expect("tick");
    }
    assert!(texts(&game).is_empty(), "{:?}", texts(&game));

    // On a chocobo Square dismounts and no moogle comes.
    game.world_play.as_mut().expect("walking").set_ride(2);
    press(&mut game, 0, 5);
    press(&mut game, pad::SQUARE, 200);
    assert!(
        !texts(&game).iter().any(|text| text.contains("kupo")),
        "{:?}",
        texts(&game)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The moogle's Save offer opens the save screen over the world map and a slot written there
/// loads back on the world map at the same spot, with the area's name in the slot row.
#[test]
fn the_moogle_saves_the_world_map_and_the_save_loads_back_there() {
    let Some((mut game, dir)) = on_the_world_map("save") else {
        return;
    };
    let spot = game.world_play.as_ref().expect("walking").walker.pos;
    open_the_save_screen(&mut game);
    // Slot 2: down once, confirm; the screen asks nothing for an empty slot.
    press(&mut game, pad::DOWN, 2);
    press(&mut game, pad::CONFIRM, 5);
    assert!(
        dir.join("slot2.json").is_file(),
        "slot 2 was written in {}",
        dir.display()
    );
    let rows = game.slot_rows();
    let SlotRow::Filled(file) = rows.get(1).expect("slot 2 row") else {
        panic!("slot 2 is empty: {rows:?}");
    };
    println!("slot row {file:?}");
    assert!(
        !file.place.is_empty() && file.place != "World Map" && file.place != "?",
        "the area's name: {}",
        file.place
    );
    // Loading it comes back on the world map on foot where the player stood.
    let mut events = Vec::new();
    game.load_slot(2, &mut events).expect("load");
    for _ in 0..5 {
        game.tick(0).expect("tick");
    }
    let play = game.world_play.as_ref().expect("on the world map");
    assert_eq!(play.ride, 0);
    assert!(
        (play.walker.pos.x - spot.x).hypot(play.walker.pos.z - spot.z) < 3.0,
        "{:?} vs {spot:?}",
        play.walker.pos
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A world map save keeps the position and the vehicle: walk away, ride the Hilda Garde III
/// (ride 8), save; the load puts the player at that spot in the airship.
#[test]
fn a_world_map_save_keeps_the_position_and_the_vehicle() {
    let Some((mut game, dir)) = on_the_world_map("trip") else {
        return;
    };
    for _ in 0..90 {
        game.tick(pad::UP).expect("tick");
    }
    game.world_play.as_mut().expect("walking").set_ride(8);
    for _ in 0..40 {
        game.tick(pad::RIGHT).expect("tick");
    }
    let spot = game.world_play.as_ref().expect("flying").walker.pos;
    game.save_slot(3).expect("save");
    let mut events = Vec::new();
    game.load_slot(1, &mut events).expect("load another spot");
    for _ in 0..5 {
        game.tick(0).expect("tick");
    }
    game.load_slot(3, &mut events).expect("load the trip");
    for _ in 0..3 {
        game.tick(0).expect("tick");
    }
    let play = game.world_play.as_ref().expect("on the world map");
    assert_eq!(play.ride, 8, "the Hilda Garde III again");
    assert!(
        (play.walker.pos.x - spot.x).hypot(play.walker.pos.z - spot.z) < 6.0,
        "{:?} vs {spot:?}",
        play.walker.pos
    );
    let _ = std::fs::remove_dir_all(&dir);
}
