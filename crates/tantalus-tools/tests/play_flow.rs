//! The play window's game flow between fields: the disc change (`DISCCHANGE`), world exits
//! and the world map menu. Real-data checks; each skips without the install (and the disc
//! one without the end-of-disc-1 save under `local/probe/play-p0/`).
use std::path::PathBuf;

use tantalus_tools::play::{DiscStage, PlayData, PlayEvent, PlayGame, PlayPaths, pad};

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

/// Presses `bits` for one tick, then releases them for one.
fn press(game: &mut PlayGame, bits: i32) -> Vec<PlayEvent> {
    let mut events = game.tick(bits).expect("tick");
    events.extend(game.tick(0).expect("tick"));
    events
}

fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("tantalus-play-flow-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// The end of disc 1 (field 70 with the disc-change script): the disc 2 picture until a
/// press, the save screen, then disc 2's first field. A save made there starts disc 2.
#[test]
fn the_end_of_disc_one_shows_the_picture_asks_to_save_and_starts_disc_two() {
    let Some(paths) = paths() else { return };
    let save =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../local/probe/play-p0/disc1-end.json");
    if !save.is_file() {
        return;
    }
    let dir = temp_dir("disc");
    std::fs::copy(&save, dir.join("slot1.json")).expect("copy save");
    let data = PlayData::load(&paths).expect("play data");
    let (mut game, _) = PlayGame::new(data, 70).expect("game");
    game.set_save_dir(dir.clone());
    let mut events = Vec::new();
    game.load_slot(1, &mut events).expect("load");
    let mut changed = None;
    for _ in 0..300 {
        for event in game.tick(0).expect("tick") {
            if let PlayEvent::DiscChange { field, disc } = event {
                changed = Some((field, disc));
            }
        }
        if changed.is_some() {
            break;
        }
    }
    assert_eq!(
        changed,
        Some((800, 1)),
        "the script changes to disc 2 at field 800"
    );
    let change = *game.disc_change().expect("disc change");
    assert_eq!(change.stage, DiscStage::Picture);
    assert_eq!(change.picture(), Some(2));
    let (picture, _) = game.disc_picture().expect("picture");
    assert!(picture.ends_with("changetodisc2.png"));
    assert!(picture.is_file(), "{}", picture.display());
    // The field is held: nothing moves on without a press.
    for _ in 0..200 {
        game.tick(0).expect("tick");
    }
    assert_eq!(game.map(), Some(70));
    assert_eq!(
        game.disc_change().map(|c| c.stage),
        Some(DiscStage::Picture)
    );
    press(&mut game, pad::CONFIRM);
    assert_eq!(
        game.disc_change().map(|c| c.stage),
        Some(DiscStage::FadeOut)
    );
    for _ in 0..60 {
        game.tick(0).expect("tick");
    }
    assert_eq!(game.disc_change().map(|c| c.stage), Some(DiscStage::Save));
    assert!(game.menu_frame().is_some(), "the save screen is open");
    // Save to slot 2 (one down from the first slot).
    press(&mut game, pad::DOWN);
    press(&mut game, pad::CONFIRM);
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("slot2.json")).expect("slot 2"))
            .expect("save json");
    assert_eq!(saved["map"], 800, "the disc-change save starts disc 2");
    assert!(saved["position"].is_null());
    // Leaving the save screen starts disc 2.
    let mut entered = Vec::new();
    entered.extend(press(&mut game, pad::CANCEL));
    entered.extend(game.tick(0).expect("tick"));
    assert!(game.disc_change().is_none());
    assert_eq!(game.map(), Some(800));
    assert!(
        entered
            .iter()
            .any(|event| matches!(event, PlayEvent::Field { map: 800, .. }))
    );
    // Loading that save starts disc 2 too.
    let mut events = Vec::new();
    game.load_slot(2, &mut events).expect("load slot 2");
    assert_eq!(game.map(), Some(800));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--world-exits` locations are used in order; once they run out the world map is walked,
/// not stopped at.
#[test]
fn world_exits_run_out_into_the_world_map() {
    let Some(paths) = paths() else { return };
    let data = PlayData::load(&paths).expect("play data");
    let (mut game, _) = PlayGame::new(data, 70).expect("game");
    game.world_mode = true;
    game.world_exits = vec![4];
    let mut events = Vec::new();
    game.leave_for_world(9000, &mut events)
        .expect("scripted exit");
    assert!(game.world_play.is_none());
    assert!(game.stopped.is_none());
    assert!(
        events.iter().any(|event| matches!(
            event,
            PlayEvent::WorldMap {
                location: Some(4),
                next: Some(_),
                ..
            }
        )),
        "{events:?}"
    );
    let mut events = Vec::new();
    game.leave_for_world(9000, &mut events).expect("world map");
    assert!(game.stopped.is_none(), "{:?}", game.stopped);
    assert!(
        game.world_play.is_some(),
        "the second exit walks the world map"
    );
}

/// The main menu opens on the world map on foot (`WorldHUD.OnKeyMenu`), works on the
/// party, has no Save there (`MainMenuUI` has no Save command), and stays shut on a chocobo.
#[test]
fn the_main_menu_opens_on_the_world_map_on_foot() {
    let Some(paths) = paths() else { return };
    let data = PlayData::load(&paths).expect("play data");
    let (mut game, _) = PlayGame::new(data, 70).expect("game");
    game.set_party(&[0, 1]);
    let mut events = Vec::new();
    game.start_world(None, &mut events).expect("world");
    for _ in 0..5 {
        game.tick(0).expect("tick");
    }
    let frames = game.world_play.as_ref().expect("walking").frames;
    press(&mut game, pad::START);
    assert!(game.menu_frame().is_some(), "START opens the main menu");
    // The world holds still while the menu is open.
    for _ in 0..10 {
        game.tick(0).expect("tick");
    }
    assert_eq!(game.world_play.as_ref().expect("walking").frames, frames);
    let text = |game: &PlayGame| {
        let (_, frame) = game.menu_frame().expect("menu");
        frame
            .windows
            .iter()
            .flat_map(|window| window.texts.iter().map(|run| run.text.clone()))
            .collect::<Vec<_>>()
            .join("|")
    };
    // Save (the last command) is refused on the world map.
    for _ in 0..7 {
        press(&mut game, pad::DOWN);
    }
    press(&mut game, pad::CONFIRM);
    assert!(text(&game).contains("save point"), "{}", text(&game));
    // Items opens.
    press(&mut game, pad::DOWN);
    press(&mut game, pad::CONFIRM);
    let before = text(&game);
    assert!(game.menu_frame().is_some());
    press(&mut game, pad::CANCEL);
    assert_ne!(text(&game), before, "back on the commands");
    press(&mut game, pad::CANCEL);
    assert!(game.menu_frame().is_none(), "cancel closes the menu");
    // On a chocobo the menu stays shut.
    game.world_play.as_mut().expect("walking").set_ride(1);
    press(&mut game, pad::START);
    assert!(game.menu_frame().is_none());
}

/// The world map plays its own song (`ff9.w_musicPlay`): Over the Hill on foot, the chocobo
/// song on a chocobo; the field song that played before is gone, and a real director plays it.
#[test]
fn the_world_map_plays_its_song_and_a_chocobo_its_own() {
    use tantalus_tools::play_audio::{AudioCue, AudioDirector};
    let Some(paths) = paths() else { return };
    let bank =
        tantalus_tools::sound_bank::SoundBank::open(&paths.streaming_assets, &paths.game_data)
            .expect("sound bank");
    // A world map fight takes its song from the world table (`WldBtlEncountBgmMetaData`).
    let fight_song =
        tantalus_tools::sound_bank::music_for_battle(&bank.world_battle_songs, 9000, 359);
    println!("world battle 359 song {fight_song}");
    assert!(fight_song >= 0);
    let data = PlayData::load(&paths).expect("play data");
    let (mut game, _) = PlayGame::new(data, 70).expect("game");
    game.set_party(&[0, 1]);
    let mut director = AudioDirector::new(bank, 6000).expect("director");
    let run = |game: &mut PlayGame, director: &mut AudioDirector| {
        let cues: Vec<AudioCue> = game.take_audio().into_iter().map(|(_, cue)| cue).collect();
        for cue in &cues {
            director.apply(cue);
        }
        let _ = director.render(2);
        cues
    };
    game.tick(0).expect("tick");
    run(&mut game, &mut director);
    let mut events = Vec::new();
    game.start_world(None, &mut events).expect("world");
    let cues = run(&mut game, &mut director);
    assert!(cues.contains(&AudioCue::WorldSong { song: 69 }), "{cues:?}");
    assert_eq!(director.system.current_music_id(), 69);
    game.world_play.as_mut().expect("walking").set_ride(1);
    game.tick(0).expect("tick");
    let cues = run(&mut game, &mut director);
    assert!(
        cues.contains(&AudioCue::WorldSong { song: 100 }),
        "{cues:?}"
    );
    assert_eq!(director.system.current_music_id(), 100);
}

/// The title screen (`TitleUI`): New Game starts the opening, Load Game opens the load list
/// (cancel goes back to the title) and Continue loads the save written last.
#[test]
fn the_title_screen_starts_a_new_game_or_continues() {
    let Some(paths) = paths() else { return };
    let data = PlayData::load(&paths).expect("play data");
    let (mut game, _) = PlayGame::new(data, 70).expect("game");
    let dir = temp_dir("title");
    game.set_save_dir(dir.clone());
    game.open_title(70);
    assert!(game.title_open());
    assert!(game.menu_frame().is_some(), "the title draws");
    // No save: the cursor starts on New Game, and Continue is refused.
    press(&mut game, pad::UP);
    press(&mut game, pad::CONFIRM);
    assert!(game.title_open(), "nothing to continue");
    press(&mut game, pad::DOWN);
    press(&mut game, pad::DOWN);
    press(&mut game, pad::CONFIRM);
    assert!(game.title_open());
    assert!(game.menu_frame().is_some(), "the load list is up");
    press(&mut game, pad::CANCEL);
    assert!(game.title_open(), "cancel goes back to the title");
    press(&mut game, pad::UP);
    let events = press(&mut game, pad::CONFIRM);
    assert!(!game.title_open(), "New Game closes the title");
    assert_eq!(game.map(), Some(70));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, PlayEvent::Field { map: 70, .. }))
    );
    // A save: Continue loads it.
    game.save_slot(3).expect("save");
    game.open_title(70);
    let events = press(&mut game, pad::CONFIRM);
    assert!(!game.title_open(), "Continue loads slot 3");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, PlayEvent::Field { .. }))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A world map save made in an airship loads in the airship: `w_frameMapConstructor` ->
/// `w_movementChange` takes the ride from gEventGlobal 190, which the save keeps.
#[test]
fn a_world_map_save_in_an_airship_loads_flying() {
    let Some(paths) = paths() else { return };
    let save = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../local/probe/play-audit/saves/slot5.json");
    if !save.is_file() {
        return;
    }
    let dir = temp_dir("airship");
    std::fs::copy(&save, dir.join("slot1.json")).expect("copy save");
    let data = PlayData::load(&paths).expect("play data");
    let (mut game, _) = PlayGame::new(data, 70).expect("game");
    game.set_save_dir(dir.clone());
    let mut events = Vec::new();
    game.load_slot(1, &mut events).expect("load");
    for _ in 0..5 {
        game.tick(0).expect("tick");
    }
    assert_eq!(game.world_play.as_ref().expect("walking").ride, 0);
    game.world_play.as_mut().expect("walking").set_ride(8);
    for _ in 0..5 {
        game.tick(0).expect("tick");
    }
    game.save_slot(2).expect("save");
    let mut events = Vec::new();
    game.load_slot(2, &mut events)
        .expect("load the airship save");
    let play = game.world_play.as_ref().expect("on the world map");
    assert_eq!(play.ride, 8, "the Hilda Garde III again");
    // On foot again, a save loads on foot.
    game.world_play.as_mut().expect("walking").set_ride(0);
    game.tick(0).expect("tick");
    game.save_slot(3).expect("save");
    game.load_slot(3, &mut events).expect("load");
    assert_eq!(game.world_play.as_ref().expect("walking").ride, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `MENU(1, character)` opens the naming screen: the field waits, typing and Enter store the
/// name, and it shows in the main menu and in a save.
#[test]
fn the_naming_screen_pauses_the_field_and_stores_the_name() {
    let Some(paths) = paths() else { return };
    let data = PlayData::load(&paths).expect("play data");
    let (mut game, _) = PlayGame::new(data, 70).expect("game");
    for _ in 0..5 {
        game.tick(0).expect("tick");
    }
    game.open_name(1);
    game.tick(0).expect("tick");
    assert!(game.name_screen_open());
    let (_, frame) = game.menu_frame().expect("name frame");
    assert!(frame.windows[0].texts.iter().any(|t| t.text == "Vivi_"));
    let before = game.world().actors.clone();
    for _ in 0..30 {
        game.tick(0).expect("tick");
    }
    assert!(game.name_screen_open(), "the screen waits for a name");
    assert!(
        game.world().actors == before,
        "the field does not run behind it"
    );
    // Empty is refused; the screen stays.
    game.name_input("", 10, true);
    assert!(game.name_screen_open());
    game.name_input("Bobo", 0, true);
    assert!(!game.name_screen_open());
    assert_eq!(
        game.world().character_names.get(&1).map(String::as_str),
        Some("Bobo")
    );
    let dir = temp_dir("name");
    game.set_save_dir(dir.clone());
    game.save_slot(1).expect("save");
    let mut events = Vec::new();
    game.load_slot(1, &mut events).expect("load");
    assert_eq!(
        game.world().character_names.get(&1).map(String::as_str),
        Some("Bobo")
    );
}
