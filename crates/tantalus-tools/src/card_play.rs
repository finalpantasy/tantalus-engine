//! Tetra Master in the play loop: the tables from the install and the sound of each event.

use std::path::Path;

use tantalus_game::cards::{CardEvent, CardTables};
use tantalus_legacy::quad::{CardTable, EnemyTables};

use crate::play_audio::AudioCue;

/// `QuadMistSoundID` effect ids (`SoundEffect.cs`).
mod sfx {
    pub const PERFECT: i32 = 1852;
    pub const CURSOR: i32 = 1853;
    pub const WARNING: i32 = 1854;
    pub const CANCEL: i32 = 1855;
    pub const CARD_GET: i32 = 1856;
    pub const CARD_MOVE: i32 = 1857;
    pub const COIN: i32 = 1858;
    pub const WALL: i32 = 1859;
    pub const FLASH: i32 = 1861;
    pub const COMB: i32 = 1862;
    pub const BOMB: i32 = 1863;
    pub const WINDOW: i32 = 1864;
    pub const WIN: i32 = 2331;
    pub const LOSE: i32 = 2332;
    pub const DRAW: i32 = 2333;
}

/// Reads the card, level and stage tables from `resources.assets` in the install's data folder.
pub fn load_tables(game_data: &Path) -> Result<CardTables, String> {
    let assets = game_data.join("resources.assets");
    let read = |name: &str| tantalus_assets::pc::audio_decode::read_text_asset(&assets, name);
    let cards = CardTable::from_bytes(&read("MINIGAME_CARD_DATA_ADDRESS")?)
        .map_err(|error| format!("card table: {error:?}"))?;
    let enemies = EnemyTables::from_bytes(
        &read("MINIGAME_CARD_LEVEL_ADDRESS")?,
        &read("MINIGAME_STAGE_ADDRESS")?,
    )
    .map_err(|error| format!("enemy tables: {error:?}"))?;
    Ok(CardTables { cards, enemies })
}

/// The audio cue of a game event.
pub fn cue(event: CardEvent) -> AudioCue {
    let id = match event {
        CardEvent::Started => return AudioCue::CardStarted,
        CardEvent::Ended => return AudioCue::CardEnded,
        CardEvent::Cursor => sfx::CURSOR,
        CardEvent::Cancel => sfx::CANCEL,
        CardEvent::Warning => sfx::WARNING,
        CardEvent::Coin => sfx::COIN,
        CardEvent::Wall => sfx::WALL,
        CardEvent::Place => sfx::CARD_MOVE,
        CardEvent::Flash => sfx::FLASH,
        CardEvent::Combo => sfx::COMB,
        CardEvent::Bomb => sfx::BOMB,
        CardEvent::Win => sfx::WIN,
        CardEvent::Perfect => sfx::PERFECT,
        CardEvent::Lose => sfx::LOSE,
        CardEvent::Draw => sfx::DRAW,
        CardEvent::CardGet => sfx::CARD_GET,
        CardEvent::CardLost => sfx::WINDOW,
    };
    AudioCue::Effect { id }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_tables_load_and_a_game_plays() {
        let Some(dir) = std::env::var_os("FFIX_GAME_DATA") else {
            return;
        };
        let tables = load_tables(std::path::Path::new(&dir)).unwrap();
        let (level, wise) = tables.enemies.stage(20);
        println!("stage 20: level {level} wise {wise}");
        let mut world = tantalus_game::FieldWorld::new();
        world.cards = (0..8).collect();
        let game = tantalus_game::cards::CardGame::start(&mut world, &tables, 20);
        assert!(game.is_some());
        assert_eq!(world.card_records.len(), 8);
    }

    #[test]
    fn start_and_end_carry_the_song() {
        assert_eq!(cue(CardEvent::Started), AudioCue::CardStarted);
        assert_eq!(cue(CardEvent::Ended), AudioCue::CardEnded);
        assert_eq!(cue(CardEvent::Win), AudioCue::Effect { id: 2331 });
    }
}
