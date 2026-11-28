//! Save files of the play window.
//!
//! The contents follow the reference's save: the field map and the
//! player's position, and the world state (party, items, gil, equipment,
//! experience, ability progress, event variables). Field scenes are not
//! saved: loading enters the map again with the saved world, as the game
//! does, and then puts the player back where they stood.
//!
//! Files are JSON, one per slot, in a folder outside the repository:
//! `%APPDATA%/tantalus/saves` (`$HOME/.tantalus/saves` elsewhere), or the
//! folder in the `TANTALUS_SAVE_DIR` environment variable.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tantalus_game::FieldWorld;

/// Format number of [`PlaySave`].
pub const SAVE_FORMAT: u32 = 1;
/// Slots in the save and load screens.
pub const SLOTS: usize = 8;

/// One save file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaySave {
    pub format: u32,
    /// Field map number to enter.
    pub map: i32,
    /// Player position when saving, `None` when there was no player.
    pub position: Option<[f32; 3]>,
    /// Game ticks played (the play time the main menu's clock and the save list show).
    #[serde(default)]
    pub ticks: u64,
    pub world: FieldWorld,
    /// The place the save screen names (`FF9.mapNameStr` when saving); older saves have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
}

/// The default save folder.
pub fn default_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TANTALUS_SAVE_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("tantalus").join("saves");
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(".tantalus").join("saves")
}

/// `slot` is 1-based.
pub fn slot_path(dir: &Path, slot: usize) -> PathBuf {
    dir.join(format!("slot{slot}.json"))
}

/// Writes a slot through a temporary file, so a failed write keeps the old
/// save.
pub fn write(dir: &Path, slot: usize, save: &PlaySave) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let text =
        serde_json::to_string(save).map_err(|error| format!("cannot encode save: {error}"))?;
    let path = slot_path(dir, slot);
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text)
        .map_err(|error| format!("cannot write {}: {error}", temp.display()))?;
    std::fs::rename(&temp, &path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))
}

/// Reads a slot: `Ok(None)` when it has no file.
pub fn read(dir: &Path, slot: usize) -> Result<Option<PlaySave>, String> {
    let path = slot_path(dir, slot);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let save: PlaySave = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not a save: {error}", path.display()))?;
    if save.format != SAVE_FORMAT {
        return Err(format!(
            "{} has save format {}, expected {SAVE_FORMAT}",
            path.display(),
            save.format
        ));
    }
    Ok(Some(save))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_game::field::world::CharacterStats;

    fn world() -> FieldWorld {
        let mut world = FieldWorld::new();
        world.party = vec![0, 1];
        world.gil = 1234;
        world.items.insert(236, 5);
        world.character_exp.insert(0, 120);
        world.character_stats.insert(
            0,
            CharacterStats {
                current_hp: 40,
                max_hp: 150,
                current_mp: 3,
                max_mp: 20,
            },
        );
        world.equip(0, 0, Some(1));
        world.support_enabled.insert(0, vec![5, 9]);
        world.apply_ap(0, 25, 80, 30);
        world.provide_atan_table(vec![1, 2, 3, 4]);
        world
    }

    #[test]
    fn a_save_loads_back_equal() {
        let dir = std::env::temp_dir().join(format!("tantalus-save-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let world = world();
        let save = PlaySave {
            format: SAVE_FORMAT,
            map: 154,
            position: Some([1.0, 2.0, 3.0]),
            ticks: 99,
            world: world.for_save(),
            place: None,
        };
        write(&dir, 3, &save).expect("write");
        let loaded = read(&dir, 3).expect("read").expect("slot exists");
        assert_eq!(loaded, save);
        // After a load the providers come back from the new-game world.
        let mut restored = loaded.world;
        restored.copy_providers_from(&world);
        assert_eq!(restored, world);
        assert!(read(&dir, 4).expect("read").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_save_without_ticks_or_place_loads() {
        let dir = std::env::temp_dir().join(format!("tantalus-save-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save = PlaySave {
            format: SAVE_FORMAT,
            map: 154,
            position: None,
            ticks: 5,
            world: world().for_save(),
            place: None,
        };
        let mut value = serde_json::to_value(&save).expect("json");
        value.as_object_mut().expect("object").remove("ticks");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(slot_path(&dir, 1), value.to_string()).expect("write");
        let loaded = read(&dir, 1).expect("read").expect("slot");
        assert_eq!((loaded.ticks, loaded.place), (0, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_file_is_an_error() {
        let dir = std::env::temp_dir().join(format!("tantalus-save-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(slot_path(&dir, 1), "{").expect("write");
        assert!(read(&dir, 1).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
