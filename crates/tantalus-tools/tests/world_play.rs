//! Walking the overworld: place events, entering a place, camera turns and
//! the random battle step. Real-data checks skip without the install.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use tantalus_tools::world_data::{WorldAssets, mesh_in_form1};
use tantalus_tools::world_places::{event_code, event_places};
use tantalus_tools::world_play::WorldPlay;

const UP: i32 = 0x10;
const R1: i32 = 0x800;
const CONFIRM: i32 = 0x20000;

fn streaming() -> Option<PathBuf> {
    let dir = std::env::var_os("TANTALUS_FFIX_STREAMING_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/StreamingAssets")
        });
    dir.join("p0data3.bin").is_file().then_some(dir)
}

#[test]
fn event_code_packs_cell_and_event_like_world_event() {
    // `ff9.WorldEvent`: 0x8000 | z << 8 & 0x3F00 | x << 2 & 0xFC | id & 3.
    assert_eq!(event_code(39, 21, 1), 0x959d);
    assert_eq!(event_code(1, 1, 1), 0x8105);
    assert_eq!(event_code(64, 64, 4), 0x8000);
}

#[test]
fn form_two_meshes_stay_out_of_the_first_form() {
    assert!(mesh_in_form1("Terrain"));
    assert!(mesh_in_form1("Sea4"));
    assert!(!mesh_in_form1("Terrain2"));
    assert!(!mesh_in_form1("Sea3_2"));
}

#[test]
fn walking_turning_and_entering_a_place_on_the_real_world() {
    let Some(dir) = streaming() else {
        eprintln!("install not found; skipping");
        return;
    };
    let assets = Arc::new(WorldAssets::load(&dir, 1).expect("world assets"));
    assert!(assets.has_block(19, 10));
    let events: BTreeMap<u16, i32> =
        event_places(&dir.join("p0data7.bin"), "EVT_WORLD_WORLD00").expect("place events");
    assert_eq!(events.get(&0x959d), Some(&2));

    // Open land: north is -z and the stick moves the actor.
    let mut play = WorldPlay::new(assets.clone(), events.clone(), 1100.0, -800.0, 1).unwrap();
    let start = play.walker.pos;
    for _ in 0..120 {
        play.tick(UP, 0, CONFIRM, false);
    }
    assert!(play.walker.pos.z < start.z - 5.0, "{:?}", play.walker.pos);

    // R1 turns the camera.
    let before = play.walker.camera.rotation;
    for _ in 0..10 {
        play.tick(R1, 0, CONFIRM, false);
    }
    assert!((play.walker.camera.rotation - before).abs() > 5.0);
    let camera = play.camera();
    assert!(camera.eye[1] > camera.aim[1] - 10.0 && camera.fov > 40.0);

    // The place under the actor, and a confirm press enters it.
    let mut play = WorldPlay::new(assets, events, 1269.0, -693.0, 1).unwrap();
    let idle = play.tick(0, 0, CONFIRM, false);
    assert_eq!(play.place, Some(2));
    assert_eq!(idle.enter, None);
    let entered = play.tick(CONFIRM, CONFIRM, CONFIRM, false);
    assert_eq!(entered.enter, Some(2));
}
