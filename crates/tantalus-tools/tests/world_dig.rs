//! Chocograph digs through the world script: the Special press, the Hot and
//! Cold dialogs, the treasure in the bag and the saved hint map. Real-data
//! checks skip without the install.
use std::path::PathBuf;
use std::sync::Arc;

use tantalus_legacy::VariableType;
use tantalus_tools::play_save::{self, PlaySave};
use tantalus_tools::world_data::WorldAssets;
use tantalus_tools::world_places::event_places;
use tantalus_tools::world_play::WorldPlay;
use tantalus_tools::world_script::{START_FRAMES, WorldScript};

/// The square button: `Control.Special`, which the script reads as 0x80000.
const SQUARE: i32 = 0x8000;
const CONFIRM: i32 = 0x20000;
/// Entry 5 function 14 for picture 1: the spot (fixed point, 256 to the
/// world unit), the Hot radius and the Found radius.
const SPOT: [f32; 2] = [264_058.0 / 256.0, -255_513.0 / 256.0];
const HOT_RADIUS: f32 = 5000.0 / 256.0;
const FOUND_RADIUS: f32 = 1400.0 / 256.0;

fn streaming() -> Option<PathBuf> {
    let dir = std::env::var_os("TANTALUS_FFIX_STREAMING_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/StreamingAssets")
        });
    dir.join("p0data3.bin").is_file().then_some(dir)
}

/// A chocobo rider with picture 1 picked, `distance` world units west of
/// its spot.
fn rider(dir: &std::path::Path, picture: i32, distance: f32) -> WorldPlay {
    let bundle = dir.join("p0data7.bin");
    let assets = Arc::new(WorldAssets::load(dir, 1).expect("world assets"));
    let events = event_places(&bundle, "EVT_WORLD_WORLD00").expect("place events");
    let mut world = tantalus_game::FieldWorld::new();
    let memory = &mut world.global_memory;
    memory.set(2, VariableType::Byte, 1);
    // Riding a chocobo (190), the kind that reaches Reef (191), the
    // pictures found (187 and up).
    memory.set(190, VariableType::Byte, 2);
    memory.set(191, VariableType::Byte, 2);
    memory.set(187, VariableType::Byte, 0xFF);
    world.hintmap_id = picture;
    let mut script = WorldScript::start(&bundle, "EVT_WORLD_WORLD00", 9000, world).unwrap();
    for _ in 0..START_FRAMES {
        script.tick(0).unwrap();
    }
    let mut play = WorldPlay::new(assets, events, SPOT[0] - distance, SPOT[1], 1).unwrap();
    play.set_script(script);
    play.set_ride(2);
    play
}

/// Presses square once, then confirms every dialog the script opens until
/// it settles; returns the text ids it showed, in order.
fn dig(play: &mut WorldPlay) -> Vec<u16> {
    let mut shown = Vec::new();
    for frame in 0..600 {
        let (held, pressed) = match frame {
            5 => (SQUARE, SQUARE),
            _ => (0, 0),
        };
        let open = play.script_dialogs();
        for (_, text) in &open {
            if !shown.contains(text) {
                shown.push(*text);
            }
        }
        // Confirm what is open every few frames, once the script has had
        // a moment to show it.
        let confirm = !open.is_empty() && frame % 6 == 0;
        play.tick(
            held | if confirm { CONFIRM } else { 0 },
            pressed | if confirm { CONFIRM } else { 0 },
            CONFIRM,
            false,
        );
    }
    shown
}

fn global(play: &WorldPlay, index: u16, kind: VariableType) -> i32 {
    play.script()
        .unwrap()
        .world()
        .global_memory
        .get(index, kind)
        .unwrap()
}

#[test]
fn square_on_the_spot_digs_up_the_treasure_and_the_hint_map_saves() {
    let Some(dir) = streaming() else { return };
    let mut play = rider(&dir, 1, 0.0);
    let before = play.script().unwrap().world().items.clone();
    let shown = dig(&mut play);
    // Found (62), then the treasure's own message (76 an item, 77 more).
    assert_eq!(shown.first(), Some(&62), "{shown:?}");
    assert!(shown.contains(&63), "{shown:?}");
    // The picture is dug up (global 184 bit 0) and the bag grew.
    assert_eq!(global(&play, 184, VariableType::Int24) & 1, 1);
    let after = play.script().unwrap().world().items.clone();
    assert_ne!(before, after, "no treasure: {shown:?}");
    // The player's world takes it, and keeps the pick for the save.
    let mut player = tantalus_game::FieldWorld::new();
    play.sync_out_of_script(&mut player);
    assert_eq!(player.items, after);
    assert_eq!(player.hintmap_id, 1);
    let dir = std::env::temp_dir().join("tantalus-dig-save");
    let _ = std::fs::create_dir_all(&dir);
    let save = PlaySave {
        format: play_save::SAVE_FORMAT,
        map: 9000,
        position: None,
        ticks: 0,
        world: player.for_save(),
    };
    play_save::write(&dir, 1, &save).unwrap();
    let back = play_save::read(&dir, 1).unwrap().unwrap();
    assert_eq!(back.world.hintmap_id, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn near_the_spot_is_hot_and_far_is_cold() {
    let Some(dir) = streaming() else { return };
    let cases = [
        (FOUND_RADIUS * 0.5, 62),
        (FOUND_RADIUS + 2.0, 61),
        (HOT_RADIUS - 1.0, 61),
        (HOT_RADIUS + 3.0, 60),
        (60.0, 60),
    ];
    for (distance, text) in cases {
        let mut play = rider(&dir, 1, distance);
        let shown = dig(&mut play);
        assert_eq!(shown.first(), Some(&text), "{distance} units: {shown:?}");
    }
}
