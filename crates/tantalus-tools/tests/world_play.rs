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

#[test]
fn every_foot_place_on_the_world_leads_to_a_field() {
    use tantalus_tools::field_session::WorldExit;
    use tantalus_tools::world_places::event_place_options;
    let Some(dir) = streaming() else {
        eprintln!("install not found; skipping");
        return;
    };
    let bundle = dir.join("p0data7.bin");
    let options = event_place_options(&bundle, "EVT_WORLD_WORLD00").expect("places");
    assert_eq!(options.len(), 27, "handlers that name a place");
    assert_eq!(options[&0x976d], vec![93, 19]);
    let exit = WorldExit::load(&bundle, "EVT_WORLD_WORLD00").expect("exit");
    let mut fieldless = Vec::new();
    let mut fields = BTreeMap::new();
    for (code, places) in &options {
        for place in places {
            let mut world = tantalus_game::field::FieldWorld::default();
            match exit.run(*place, &mut world) {
                Ok(Some(field)) => {
                    fields.insert(*place, field);
                }
                _ => fieldless.push((*code, *place)),
            }
        }
    }
    // Places 92 and 93 start scripted scenes on the world map and name no field.
    assert_eq!(fieldless, vec![(0x976d, 93), (0x9799, 92)]);
    assert_eq!(fields.len(), 26);
    // The chocobo grounds and the corner site come from the later branches.
    assert_eq!(fields[&20], 2950);
    assert_eq!(fields[&50], 2954);
    assert_eq!(fields[&2], 1856);
}

/// `EVT_WORLD_WORLD02` (disc 2, world map 9002) reads the place from map
/// byte 30, not 29: place 6 is Treno's gate (field 908) before scenario 6990.
#[test]
fn the_disc_two_world_reads_its_own_place_variable() {
    use tantalus_tools::field_session::WorldExit;
    let Some(dir) = streaming() else {
        eprintln!("install not found; skipping");
        return;
    };
    let exit = WorldExit::load(&dir.join("p0data7.bin"), "EVT_WORLD_WORLD02").expect("exit");
    let mut world = tantalus_game::field::FieldWorld::default();
    assert_eq!(exit.run(6, &mut world), Ok(Some(908)));
    let mut world = tantalus_game::field::FieldWorld::default();
    assert_eq!(exit.run(19, &mut world), Ok(Some(1000)));
}

#[test]
fn the_world_wraps_and_blocks_take_their_second_form() {
    use tantalus_legacy::world::{RaycastMode, Vec3, wrap_position};
    use tantalus_tools::world_data::{alternate_form_blocks, mesh_in_form};
    let wrapped = wrap_position(Vec3::new(-10.0, 5.0, 20.0));
    assert_eq!((wrapped.x, wrapped.z), (1526.0, -1260.0));
    let inside = wrap_position(Vec3::new(1264.0, 5.0, -688.0));
    assert_eq!((inside.x, inside.z), (1264.0, -688.0));

    assert!(alternate_form_blocks(2000, 0).is_empty());
    assert!(alternate_form_blocks(3000, 0).contains(&(18, 14)));
    assert!(alternate_form_blocks(9000, 0x40).contains(&(19, 10)));
    assert!(alternate_form_blocks(9000, 0x40).contains(&(16, 14)));
    assert!(mesh_in_form("Terrain", 1, (3, 9)) && !mesh_in_form("Terrain2", 1, (3, 9)));
    assert!(mesh_in_form("Terrain2", 2, (3, 9)) && !mesh_in_form("Terrain", 2, (3, 9)));
    assert!(!mesh_in_form("Sea3", 2, (3, 9)) && mesh_in_form("Sea3", 2, (4, 4)));

    let Some(dir) = streaming() else {
        eprintln!("install not found; skipping");
        return;
    };
    let assets = Arc::new(WorldAssets::load(&dir, 1).expect("world assets"));
    let mode = RaycastMode::default();
    let at = |x: f32| assets.walk.nwp_hit(Vec3::new(x, 30.0, -688.0), &mode, None);
    let (home, far) = (at(1264.0), at(1264.0 - 1536.0));
    assert_eq!(home.pno, 1000);
    assert_eq!((far.pno, far.height), (home.pno, home.height));
    // A block with a second form walks on it once switched.
    let mut play = WorldPlay::new(assets.clone(), BTreeMap::new(), 1264.0, -688.0, 1).unwrap();
    play.set_forms(&[(18, 14), (0, 0)]);
    assert!(play.forms.contains(&(18, 14)));
}

#[test]
fn chocobos_and_the_airship_move_the_player() {
    let Some(dir) = streaming() else {
        eprintln!("install not found; skipping");
        return;
    };
    let assets = Arc::new(WorldAssets::load(&dir, 1).expect("world assets"));
    let run = |ride: u8, held: i32| {
        let mut play = WorldPlay::new(assets.clone(), BTreeMap::new(), 1100.0, -800.0, 1).unwrap();
        play.set_ride(ride);
        assert_eq!(play.ride, ride);
        let start = play.walker.pos;
        for _ in 0..60 {
            play.tick(held, 0, CONFIRM, false);
        }
        (play.walker.pos.z - start.z).abs()
    };
    let foot = run(0, UP);
    assert!(run(1, UP) > foot * 1.3, "yellow chocobo");
    for kind in 2..=5 {
        assert!(run(kind, UP) > 0.0, "chocobo kind {kind} moves");
    }
    // The gold chocobo flies: throttle moves it and L2 climbs.
    let air = run(6, UP);
    assert!(air > foot, "gold chocobo {air} foot {foot}");
    let mut play = WorldPlay::new(assets.clone(), BTreeMap::new(), 1100.0, -800.0, 1).unwrap();
    play.set_ride(6);
    assert!(play.flying());
    let ground = play.walker.pos.y;
    for _ in 0..30 {
        play.tick(0x100, 0, CONFIRM, false);
    }
    assert!(play.walker.pos.y > ground, "climb");
    // The Blue Narciss casts off from the sea only, and sails on it.
    let mut land = WorldPlay::new(assets.clone(), BTreeMap::new(), 1100.0, -800.0, 1).unwrap();
    land.set_ride(7);
    assert_eq!(land.ride, 0, "no ship on dry land");
    let mut ship = WorldPlay::new(assets, BTreeMap::new(), 40.0, -400.0, 1).unwrap();
    ship.set_ride(7);
    assert!(ship.sailing());
    let start = ship.walker.pos;
    for _ in 0..90 {
        ship.tick(UP, 0, CONFIRM, false);
    }
    let moved = (ship.walker.pos.z - start.z).hypot(ship.walker.pos.x - start.x);
    assert!(moved > 5.0, "the ship sails {moved}");
    let play = &mut play;
    // Getting off: from a chocobo onto the ground, from the open sea not at
    // all, and from the ship at the shore onto land; the ship stays anchored.
    let mut rider = WorldPlay::new(
        std::sync::Arc::clone(&ship.assets),
        BTreeMap::new(),
        1100.0,
        -800.0,
        1,
    )
    .unwrap();
    rider.set_ride(1);
    assert!(rider.get_off(), "off the chocobo");
    assert_eq!(rider.ride, 0);
    assert!(!rider.get_off(), "nothing to get off");
    assert!(!ship.get_off(), "no shore in the open sea");
    assert!(ship.sailing());
    let mut landed = false;
    for x in (0..1500).step_by(4) {
        ship.walker.pos.x = x as f32;
        ship.walker.pos.z = -400.0;
        ship.set_ride(0);
        ship.set_ride(7);
        if ship.sailing() && ship.get_off() {
            assert!(ship.moored.is_some());
            assert!(!ship.sailing());
            let anchor = ship.moored.unwrap();
            ship.walker.pos = anchor;
            assert!(ship.board(), "back on board");
            assert!(ship.sailing());
            landed = true;
            break;
        }
    }
    assert!(landed, "the ship finds a shore");
    // Wrapping keeps the flight inside the world.
    play.set_ride(0);
    assert!(!play.flying());
}

#[test]
fn the_world_script_stands_the_chocobo_at_its_story_point_spot() {
    let Some(dir) = streaming() else { return };
    let bundle = dir.join("p0data7.bin");
    let assets = Arc::new(WorldAssets::load(&dir, 1).expect("world assets"));
    let events = event_places(&bundle, "EVT_WORLD_WORLD00").expect("place events");
    for (story, x, z, facing) in [
        (0, 243_177.0, -251_993.0, 77.0),
        (11_100, 279_017.0, -248_612.0, 250.0),
    ] {
        let mut world = tantalus_game::FieldWorld::new();
        world
            .global_memory
            .set(0, tantalus_legacy::VariableType::UInt16, story);
        world
            .global_memory
            .set(191, tantalus_legacy::VariableType::Byte, 2);
        let mut script = tantalus_tools::world_script::WorldScript::start(
            &bundle,
            "EVT_WORLD_WORLD00",
            9000,
            world,
        )
        .unwrap();
        for _ in 0..tantalus_tools::world_script::START_FRAMES {
            script.tick(0).unwrap();
        }
        let mut play = WorldPlay::new(assets.clone(), events.clone(), 0.0, 0.0, 1).unwrap();
        play.set_script(script);
        let chocobo = play.chocobo.expect("a chocobo");
        eprintln!("story {story}: {chocobo:?}");
        assert!((chocobo.pos.x - x / 256.0).abs() < 0.5);
        assert!((chocobo.pos.z - z / 256.0).abs() < 0.5);
        assert!((chocobo.rot_y - facing / 256.0 * 360.0).abs() < 0.01);
        assert_eq!(chocobo.kind, 2);
        let start = play.script_start().unwrap();
        assert_eq!(start.ride, 0);
        assert!(start.zidane.is_some());
    }
}

#[test]
fn the_world_script_follows_the_walker_and_reads_the_world() {
    let Some(dir) = streaming() else { return };
    let bundle = dir.join("p0data7.bin");
    let assets = Arc::new(WorldAssets::load(&dir, 1).expect("world assets"));
    let events = event_places(&bundle, "EVT_WORLD_WORLD00").expect("place events");
    let world = tantalus_game::FieldWorld::new();
    let mut script =
        tantalus_tools::world_script::WorldScript::start(&bundle, "EVT_WORLD_WORLD00", 9000, world)
            .unwrap();
    for _ in 0..tantalus_tools::world_script::START_FRAMES {
        script.tick(0).unwrap();
    }
    let mut play = WorldPlay::new(assets, events, 1218.0, -747.0, 1).unwrap();
    play.set_script(script);
    let mut battles = 0;
    for frame in 0..400 {
        let held = if frame > 100 { UP } else { 0 };
        let tick = play.tick(held, 0, CONFIRM, false);
        battles += usize::from(tick.script_battle.is_some());
        assert_eq!(tick.field, None);
    }
    let script = play.script().unwrap();
    let globals = &script.world().global_memory;
    // The script's saved position (globals 64, 67, 69) is the walker's.
    let x = globals
        .get(64, tantalus_legacy::VariableType::Int24)
        .unwrap();
    let z = globals
        .get(69, tantalus_legacy::VariableType::Int24)
        .unwrap();
    eprintln!(
        "walker {:?}; saved ({x}, {z}); effects {:?}; script battles {battles}",
        play.walker.pos, script.effects
    );
    assert!((x as f32 / 256.0 - play.walker.pos.x).abs() < 2.0);
    assert!((z as f32 / 256.0 - play.walker.pos.z).abs() < 2.0);
}

#[test]
fn the_chocograph_list_opens_from_the_script_and_picks_the_hint_map() {
    let Some(dir) = streaming() else { return };
    let bundle = dir.join("p0data7.bin");
    let assets = Arc::new(WorldAssets::load(&dir, 1).expect("world assets"));
    let events = event_places(&bundle, "EVT_WORLD_WORLD00").expect("place events");
    let mut world = tantalus_game::FieldWorld::new();
    let memory = &mut world.global_memory;
    // Pictures 0 and 6 found, the chocobo reaches Reef (ability 2).
    memory.set(187, tantalus_legacy::VariableType::Byte, 0b0100_0001);
    memory.set(191, tantalus_legacy::VariableType::Byte, 2);
    let script =
        tantalus_tools::world_script::WorldScript::start(&bundle, "EVT_WORLD_WORLD00", 9000, world)
            .unwrap();
    let mut play = WorldPlay::new(assets, events, 1218.0, -747.0, 1).unwrap();
    play.set_script(script);
    assert!(play.chocograph.is_none());
    // `OpenMenu` 5 from the script's Chocograph prompt.
    play.script_mut_for_tests().world_mut().chocograph_requested = true;
    play.tick(0, 0, CONFIRM, false);
    let book = play.chocograph.as_ref().expect("the list opens");
    assert_eq!((book.map_count(), book.ability()), (2, 2));
    assert!(play.chocograph_select(1).is_err());
    play.chocograph_select(6).unwrap();
    assert_eq!(play.hintmap, 7);
    assert!(play.chocograph_cancel());
    assert_eq!(play.hintmap, 0);
}

/// The Hilda Garde III and the Invincible (`w_moveCHRControl` 8 and 9): the
/// player boards a parked airship next to it, flies it with the plane
/// controls, lands it on flat ground (`w_movementGetGetoff`, cases 8 and 9)
/// and finds it parked where it came down.
#[test]
fn airships_board_fly_and_land() {
    use tantalus_tools::world_script::{ParkedShip, Ship};
    let Some(dir) = streaming() else {
        eprintln!("install not found; skipping");
        return;
    };
    let assets = Arc::new(WorldAssets::load(&dir, 1).expect("world assets"));
    for (ship, control) in [(Ship::HildaGarde3, 8u8), (Ship::Invincible, 9)] {
        let mut play = WorldPlay::new(assets.clone(), BTreeMap::new(), 1100.0, -800.0, 1).unwrap();
        let here = play.walker.pos;
        play.ships.push(ParkedShip {
            ship,
            pos: [here.x + 4.0, here.y, here.z],
            facing: 0,
        });
        assert_eq!(play.parked_ships().count(), 1);
        // Too far away: nothing to board.
        play.walker.pos.x += 40.0;
        assert!(!play.board(), "{ship:?} out of reach");
        play.walker.pos = here;
        assert!(play.board(), "{ship:?} boards");
        assert_eq!(play.ride, control);
        assert!(play.flying());
        assert_eq!(
            play.parked_ships().count(),
            0,
            "the flown ship is not parked"
        );
        let start = play.walker.pos;
        for _ in 0..90 {
            play.tick(UP, 0, CONFIRM, false);
        }
        let flown = (play.walker.pos.x - start.x).hypot(play.walker.pos.z - start.z);
        assert!(flown > 10.0, "{ship:?} flies {flown}");
        // Find flat ground to come down on.
        let mut landed = None;
        for step in 0..400 {
            play.walker.pos.x = 900.0 + step as f32 * 2.0;
            play.walker.pos.z = -800.0;
            let at = play.walker.pos;
            if play.get_off() {
                landed = Some(at);
                break;
            }
        }
        let at = landed.unwrap_or_else(|| panic!("{ship:?} finds somewhere to land"));
        assert_eq!(play.ride, 0);
        let parked: Vec<_> = play.parked_ships().copied().collect();
        assert_eq!(parked.len(), 1);
        assert_eq!(parked[0].ship, ship);
        assert!((parked[0].pos[0] - at.x).abs() < 1e-3 && (parked[0].pos[2] - at.z).abs() < 1e-3);
        let off = (play.walker.pos.x - at.x).hypot(play.walker.pos.z - at.z);
        assert!(off > 0.5, "the player steps off beside the ship ({off})");
        assert!(play.board(), "{ship:?} boards again where it landed");
        assert_eq!(play.ride, control);
    }
}
