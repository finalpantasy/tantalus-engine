//! The world a field exit enters follows the story point: the disc whose
//! blocks load (`WorldConfiguration.GetDisc`) and the world script the field
//! asked for. Real-data check; skips without the install.
use std::path::PathBuf;

use tantalus_tools::play::{PlayData, PlayGame, PlayPaths};

#[test]
fn world_exit_on_late_disc_loads_disc_four_and_its_script() {
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
    let (mut game, _) = PlayGame::new(data, 70).expect("game");
    let mut events = Vec::new();
    // Disc 2 (before 11090): the first disc's blocks, the script asked for.
    game.set_story(10500, 0, 0);
    game.world_script_map = 9003;
    game.start_world(None, &mut events).expect("world");
    let play = game.world_play.as_ref().expect("walking");
    assert_eq!(play.assets.disc(), 1);
    // Disc 4 (11090 and up): the Invincible's world, on disc 4 blocks.
    game.set_story(11100, 0, 0);
    game.world_script_map = 9009;
    game.start_world(None, &mut events).expect("world");
    let play = game.world_play.as_ref().expect("walking");
    assert_eq!(play.assets.disc(), 4);
    assert!(
        play.parked_ships()
            .any(|parked| parked.ship == tantalus_tools::world_script::Ship::Invincible)
    );
}
