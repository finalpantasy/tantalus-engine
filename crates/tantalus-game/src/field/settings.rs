//! The Config screen's values, kept in the save (`FF9CFG` plus the `Memoria.ini`
//! options the screen edits). The rules for reading and writing them live in
//! `tantalus_legacy::party_menu::config`; this is the serializable record.

use serde::{Deserialize, Serialize};
use tantalus_legacy::party_menu::config::{Cfg, IniOptions, Settings};

/// What the Config screen stores. Defaults are `FF9CFG()` and the `Memoria.ini`
/// defaults, so a save without the block loads as a new game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameSettings {
    /// 0 = the battle menu starts on the first command, 1 = it remembers.
    pub cursor: u64,
    /// 1 = the command menu stalls the ATB gauges (Wait), 0 = Active.
    pub atb: u64,
    pub camera: u64,
    pub move_: u64,
    pub vibe: u64,
    /// ATB coefficient selector 0..=2.
    pub btl_speed: u64,
    pub skip_btl_camera: u64,
    /// Field message speed, an index into `DIALOG_TEXT_TICK`.
    pub fld_msg: u64,
    pub here_icon: u64,
    pub win_type: u64,
    /// 0 to 100 in steps of 5.
    pub sound_volume: i32,
    pub music_volume: i32,
    pub movie_volume: i32,
    /// `[Battle] Speed`: 0 normal, 1 fast, 2 turn-based, 5 simultaneous.
    pub atb_mode: i32,
}

impl Default for GameSettings {
    fn default() -> Self {
        Settings::default().into()
    }
}

impl From<Settings> for GameSettings {
    fn from(settings: Settings) -> Self {
        let Settings { cfg, ini } = settings;
        Self {
            cursor: cfg.cursor,
            atb: cfg.atb,
            camera: cfg.camera,
            move_: cfg.move_,
            vibe: cfg.vibe,
            btl_speed: cfg.btl_speed,
            skip_btl_camera: cfg.skip_btl_camera,
            fld_msg: cfg.fld_msg,
            here_icon: cfg.here_icon,
            win_type: cfg.win_type,
            sound_volume: ini.sound_volume,
            music_volume: ini.music_volume,
            movie_volume: ini.movie_volume,
            atb_mode: ini.atb_mode,
        }
    }
}

impl GameSettings {
    /// The legacy record the Config rules work on.
    pub fn to_legacy(&self) -> Settings {
        Settings {
            cfg: Cfg {
                cursor: self.cursor,
                atb: self.atb,
                camera: self.camera,
                move_: self.move_,
                vibe: self.vibe,
                btl_speed: self.btl_speed,
                skip_btl_camera: self.skip_btl_camera,
                fld_msg: self.fld_msg,
                here_icon: self.here_icon,
                win_type: self.win_type,
                ..Cfg::default()
            },
            ini: IniOptions {
                sound_volume: self.sound_volume,
                music_volume: self.music_volume,
                movie_volume: self.movie_volume,
                atb_mode: self.atb_mode,
                ..IniOptions::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_match_the_reference_and_round_trip() {
        let settings = GameSettings::default();
        assert_eq!(
            (settings.atb, settings.btl_speed, settings.fld_msg),
            (1, 1, 3)
        );
        assert_eq!((settings.music_volume, settings.sound_volume), (100, 100));
        let back: GameSettings = settings.to_legacy().into();
        assert_eq!(back, settings);
        let text = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<GameSettings>("{}").unwrap(),
            settings
        );
        assert_eq!(
            serde_json::from_str::<GameSettings>(&text).unwrap(),
            settings
        );
    }
}
