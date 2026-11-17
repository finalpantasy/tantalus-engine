//! The world script run by the field VM, against the real install (skipped
//! without it).
use std::path::PathBuf;

use tantalus_tools::world_places::event_place_options;
use tantalus_tools::world_script::WorldScript;

fn bundle() -> Option<PathBuf> {
    let dir = std::env::var_os("TANTALUS_FFIX_STREAMING_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/StreamingAssets")
        });
    let bundle = dir.join("p0data7.bin");
    bundle.is_file().then_some(bundle)
}

fn start(bundle: &std::path::Path) -> WorldScript {
    WorldScript::start(
        bundle,
        "EVT_WORLD_WORLD00",
        9000,
        tantalus_game::FieldWorld::new(),
    )
    .unwrap()
}

#[test]
fn the_world_script_starts_and_sets_the_world_codes() {
    let Some(bundle) = bundle() else { return };
    let mut script = start(&bundle);
    for _ in 0..120 {
        script.tick(0).unwrap();
    }
    // Function 0 runs `RunWorldCode(2, 1)` (minimap on) and `(0, 4)`
    // (automatic weather) as it starts.
    eprintln!("effects {:?}", script.effects);
    assert!(script.effects.navi_active);
    assert!(script.effects.weather_auto);
}

#[test]
fn every_place_handler_run_by_the_vm_picks_the_place_the_static_scan_found() {
    let Some(bundle) = bundle() else { return };
    let options = event_place_options(&bundle, "EVT_WORLD_WORLD00").unwrap();
    let mut differing = Vec::new();
    for (code, places) in &options {
        let mut script = start(&bundle);
        for _ in 0..10 {
            script.tick(0).unwrap();
        }
        assert!(script.request_event(*code).unwrap(), "{code:#x}");
        for _ in 0..10 {
            script.tick(0).unwrap();
        }
        let place = script.place();
        if !places.contains(&place) {
            differing.push((*code, place, places.clone()));
        }
    }
    eprintln!("{} handlers; differing {differing:?}", options.len());
    assert!(options.len() > 20);
    // Three handlers (the chocobo grounds, 0x8105-style cells) decline until
    // a story or Chocograph condition holds (global bits 1468 and 1471,
    // the chocobo ability in 190); a fresh world meets none of them, which
    // the static scan, reading only the first place each can write, could
    // not know.
    assert!(
        differing.iter().all(|(_, place, _)| *place == 0) && differing.len() <= 3,
        "{differing:?}"
    );
}

#[test]
fn the_real_enter_path_leaves_for_the_same_fields_as_the_stand_in() {
    use tantalus_tools::field_session::WorldExit;
    let Some(bundle) = bundle() else { return };
    let exit = WorldExit::load(&bundle, "EVT_WORLD_WORLD00").unwrap();
    let mut differing = Vec::new();
    let mut same = 0;
    for place in 1..=100 {
        let mut old_world = tantalus_game::FieldWorld::new();
        let old = exit.run(place, &mut old_world).ok().flatten();
        let mut script = start(&bundle);
        for _ in 0..5 {
            script.tick(0).unwrap();
        }
        let new = script.enter_place(place, 600).unwrap_or(None);
        if old == new {
            same += 1;
        } else {
            differing.push((place, old, new));
        }
    }
    eprintln!("same {same}; differing {differing:?}");
    // The three chocobo grounds (places 20, 38, 50) decline on a fresh world:
    // their prompt needs the chocobo ability, which the stand-in forced.
    let places: Vec<i32> = differing.iter().map(|(place, _, _)| *place).collect();
    assert_eq!(places, [20, 38, 50]);
    assert!(differing.iter().all(|(_, _, new)| new.is_none()));
    assert_eq!(same, 97);
}

fn story_start(bundle: &std::path::Path, story: i32) -> ([f32; 3], u8) {
    let mut world = tantalus_game::FieldWorld::new();
    world
        .global_memory
        .set(0, tantalus_legacy::VariableType::UInt16, story);
    // A chocobo kind owned, so the script creates the chocobo object.
    world
        .global_memory
        .set(191, tantalus_legacy::VariableType::Byte, 1);
    let mut script = WorldScript::start(bundle, "EVT_WORLD_WORLD00", 9000, world).unwrap();
    for _ in 0..10 {
        script.tick(0).unwrap();
    }
    script.chocobo_start().unwrap()
}

#[test]
fn the_script_places_the_chocobo_by_story_point() {
    let Some(bundle) = bundle() else { return };
    let (start, facing) = story_start(&bundle, 0);
    eprintln!("story 0: {start:?} facing {facing}");
    // The script's new-game defaults: 243177, -1039, -251993 and facing 77.
    assert!((start[0] - 243_177.0 / 256.0).abs() < 0.5);
    assert!((start[2] - -251_993.0 / 256.0).abs() < 0.5);
    assert_eq!(facing, 77);
    // Story point 11100 moves it (the script's own branch for that point).
    let (narciss, facing) = story_start(&bundle, 11_100);
    eprintln!("story 11100: {narciss:?} facing {facing}");
    assert!((narciss[0] - 279_017.0 / 256.0).abs() < 0.5);
    assert!((narciss[2] - -248_612.0 / 256.0).abs() < 0.5);
    assert_eq!(facing, 250);
}
