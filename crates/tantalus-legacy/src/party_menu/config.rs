//! Config menu values, their stored form and new-game defaults.
//!
//! Source: `Global/ff9/FF9CFG.cs` (the stored record and its defaults),
//! `Global/SettingsState.cs` (`cfg = new FF9CFG()` on `Initial`), and
//! `Global/Config/ConfigUI.cs` (what each row reads and writes). Consumers of
//! each value are cited on the field.
//!
//! Not modeled: key bindings beyond the stored default (`control_data_*`
//! hold raw key codes and strings; the menu rebinding flow is UI), the
//! cheat rows (`Configuration.Cheats.*`), the Title and Quit rows, the
//! tutorials, and Memoria's `Memoria.ini` writes (`Save*Volume`).

use crate::saves::ff9_tree::SettingSection;

/// `HonoInputManager.DefaultInputKeysCount` (also `SettingSection::KEYS`).
pub const INPUT_KEYS: usize = SettingSection::KEYS;

/// `FF9CFG.FF9CFG_VIBE_ON` (`FF9CFG.cs:50`): the field is zero-initialized.
pub const VIBE_ON: u64 = 0;
/// `FF9CFG.FF9CFG_VIBE_OFF` (`FF9CFG.cs:51`).
pub const VIBE_OFF: u64 = 1;

/// Field-message slider step (`ConfigUI.cs:211`, `fieldMessageSliderStep = 6f`).
pub const FIELD_MESSAGE_STEP: f32 = 6.0;
/// Battle-speed slider step (`ConfigUI.cs:212`, `battleSpeedSliderStep = 2f`).
pub const BATTLE_SPEED_STEP: f32 = 2.0;

/// `Dialog.DialogTextAnimationTick` (`Global/Dialog/Dialog.cs:1684-1695`),
/// indexed by `cfg.fld_msg` (`FFIXTextModifier.cs:58`).
pub const DIALOG_TEXT_TICK: [u8; 8] = [4, 7, 10, 16, 24, 40, 64, 64];
/// `BattleHUD.BattleMessageTimeTick` (`BattleHUD.Const.cs:34`), indexed by
/// `cfg.btl_msg` (`BattleHUD.Unity.cs:161,168`); the counter is compared
/// against `tick / 15.0`.
pub const BATTLE_MESSAGE_TICK: [u8; 7] = [54, 46, 48, 30, 24, 18, 12];

/// `FF9CFG` (`Global/ff9/FF9CFG.cs`), all scalars `UInt64`.
///
/// What the values mean is read from the code that consumes them, not from
/// the (prefab-only) button labels: a field documents only the consumer
/// branches the source proves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cfg {
    /// 0 = music enabled (`FF9CFG.IsMusicEnabled`, `SettingsState.SetSound`).
    pub sound: u64,
    /// 0 = sound effects enabled (`IsSoundEnabled`).
    pub sound_effect: u64,
    /// 0 = default key map (`HonoInputManager.SetPrimaryKeys`,
    /// `HonoInputManager.cs:949`); other = the saved custom keys.
    pub control: u64,
    /// 0 = the battle menu starts on the first command; nonzero = it
    /// remembers the last cursor (`BattleHUD.cs:1890,1905,1937`).
    pub cursor: u64,
    /// 1 = the battle command menu stalls the ATB gauges
    /// (`BattleHUD.Public.cs:800`, `IsNativeEnableAtb`); any other value
    /// leaves them running.
    pub atb: u64,
    /// 1 = never play the alternate battle camera for an action
    /// (`SFX.cs:2311`); battle script variable 111 reads this raw
    /// (`SFX.cs:877-878`).
    pub camera: u64,
    /// 1 = running is the default and the cancel key walks
    /// (`FieldMapActorController.cs:198,732`: `Cancel key ^ move == 1`).
    pub move_: u64,
    /// [`VIBE_ON`] or [`VIBE_OFF`] (`vib.cs:168`, `MBG.cs:351`).
    pub vibe: u64,
    /// ATB coefficient selector: 0 gives 8, 1 gives 10, 2 gives 14, anything
    /// else 10 (`btl_para.GetATBCoef`, `btl_para.cs:66-76`).
    pub btl_speed: u64,
    /// 0 = random-encounter battles skip the delayed fade
    /// (`battle.cs:491`, `HonoluluBattleMain.cs:150`: `noDelayFade`).
    pub skip_btl_camera: u64,
    /// Index into [`BATTLE_MESSAGE_TICK`]. Never saved: the loader has no
    /// `btl_msg` key, so it stays at the constructor value after a load.
    pub btl_msg: u64,
    /// Index into [`DIALOG_TEXT_TICK`].
    pub fld_msg: u64,
    /// 0 = the field "here" icon shows (`EIcon.cs:351`: `num = here_icon > 0 ? 0 : 1`).
    pub here_icon: u64,
    /// 0 = gray window skin, nonzero = blue (`FF9UIDataTool.cs:273`,
    /// `TimerUI.cs:80`); also copied to the save preview
    /// (`SharedDataBytesStorage.cs:505`).
    pub win_type: u64,
    /// Unused in the pinned source (comment in `FF9CFG.cs:44`).
    pub target_win: u64,
    /// Unused in the pinned source (comment in `FF9CFG.cs:45`).
    pub control_data: u64,
    /// `KeyCode[]`, `DefaultInputKeysCount` entries, all zero in the constructor.
    pub control_data_keyboard: [i32; INPUT_KEYS],
    /// `String[]`, `DefaultInputKeysCount` entries, all empty in the constructor.
    pub control_data_joystick: Vec<String>,
}

impl Default for Cfg {
    /// `FF9CFG()` (`FF9CFG.cs:6-27`). A new game runs it through
    /// `SettingsState.Initial` (`SettingsState.cs:130`).
    fn default() -> Self {
        Self {
            sound: 0,
            sound_effect: 0,
            control: 0,
            cursor: 0,
            atb: 1,
            camera: 0,
            move_: 1,
            vibe: VIBE_ON,
            btl_speed: 1,
            skip_btl_camera: 1,
            btl_msg: 3,
            fld_msg: 3,
            here_icon: 0,
            win_type: 0,
            target_win: 0,
            control_data: 0,
            control_data_keyboard: [0; INPUT_KEYS],
            control_data_joystick: vec![String::new(); INPUT_KEYS],
        }
    }
}

impl Cfg {
    /// `btl_para.GetATBCoef` (`btl_para.cs:66-76`).
    pub fn atb_coef(&self) -> i32 {
        match self.btl_speed {
            0 => 8,
            1 => 10,
            2 => 14,
            _ => 10,
        }
    }

    /// `Dialog.DialogTextAnimationTick[cfg.fld_msg]` (`FFIXTextModifier.cs:58`);
    /// `None` past the table (the source would throw).
    pub fn field_message_tick(&self) -> Option<u8> {
        DIALOG_TEXT_TICK
            .get(usize::try_from(self.fld_msg).ok()?)
            .copied()
    }

    /// `BattleMessageTimeTick[cfg.btl_msg]` (`BattleHUD.Unity.cs:161`).
    pub fn battle_message_tick(&self) -> Option<u8> {
        BATTLE_MESSAGE_TICK
            .get(usize::try_from(self.btl_msg).ok()?)
            .copied()
    }

    /// Reads the saved `cfg` block (`JsonParser.cs:1485-1510`). The tree
    /// carries no `btl_msg`, so it keeps the constructor value.
    pub fn from_setting_section(s: &SettingSection) -> Self {
        let mut keyboard = [0i32; INPUT_KEYS];
        for (dst, src) in keyboard.iter_mut().zip(&s.control_data_keyboard) {
            *dst = *src;
        }
        let mut joystick = vec![String::new(); INPUT_KEYS];
        for (dst, src) in joystick.iter_mut().zip(&s.control_data_joystick) {
            dst.clone_from(src);
        }
        Self {
            sound: s.sound,
            sound_effect: s.sound_effect,
            control: s.control,
            cursor: s.cursor,
            atb: s.atb,
            camera: s.camera,
            move_: s.move_,
            vibe: s.vibe,
            btl_speed: s.btl_speed,
            skip_btl_camera: s.skip_btl_camera,
            btl_msg: Self::default().btl_msg,
            fld_msg: s.fld_msg,
            here_icon: s.here_icon,
            win_type: s.win_type,
            target_win: s.target_win,
            control_data: s.control_data,
            control_data_keyboard: keyboard,
            control_data_joystick: joystick,
        }
    }

    /// Writes the values back into a save tree section (keeps its `time`).
    pub fn write_setting_section(&self, s: &mut SettingSection) {
        s.sound = self.sound;
        s.sound_effect = self.sound_effect;
        s.control = self.control;
        s.cursor = self.cursor;
        s.atb = self.atb;
        s.camera = self.camera;
        s.move_ = self.move_;
        s.vibe = self.vibe;
        s.btl_speed = self.btl_speed;
        s.fld_msg = self.fld_msg;
        s.here_icon = self.here_icon;
        s.win_type = self.win_type;
        s.target_win = self.target_win;
        s.control_data = self.control_data;
        s.skip_btl_camera = self.skip_btl_camera;
        s.control_data_keyboard = self.control_data_keyboard.to_vec();
        s.control_data_joystick = self.control_data_joystick.clone();
    }
}

/// `Memoria.ini` values the Config screen edits that are not part of
/// `FF9CFG` (`Memoria/Configuration/Structure/AudioSection.cs:18-20`,
/// `BattleSection.cs:63`, `VoiceActingSection.cs:21,24`). They live in the
/// ini file, not in the save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IniOptions {
    /// 0 to 100, steps of 5 from the slider.
    pub sound_volume: i32,
    pub music_volume: i32,
    pub movie_volume: i32,
    pub voice_volume: i32,
    /// `[Battle] Speed`, default 0 (`Memoria.ini:177`): 0 normal, 1 fast,
    /// 2 turn-based, 3 to 5 simultaneous variants. The menu slider stores 5
    /// for its fourth stop (`ConfigUI.cs:1391-1392`).
    pub atb_mode: i32,
    /// `AutoDismissDialogAfterCompletion`, default false.
    pub auto_dismiss_dialog: bool,
}

impl Default for IniOptions {
    fn default() -> Self {
        Self {
            sound_volume: 100,
            music_volume: 100,
            movie_volume: 100,
            voice_volume: 100,
            atb_mode: 0,
            auto_dismiss_dialog: false,
        }
    }
}

/// `ConfigUI.Configurator` (`ConfigUI.cs:48-74`), in enum order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Configurator {
    Sound,
    SoundEffect,
    Controller,
    Cursor,
    Atb,
    BattleCamera,
    SkipBattleCamera,
    Movement,
    FieldMessage,
    BattleSpeed,
    HereIcon,
    WindowColor,
    Vibration,
    ControlTutorial,
    CombatTutorial,
    Title,
    QuitGame,
    SoundVolume,
    MusicVolume,
    MovieVolume,
    VoiceVolume,
    AtbMode,
    AutoText,
}

/// `ConfigUI.ConfigSliderIdList` (`ConfigUI.cs:165-176`).
pub fn is_slider(c: Configurator) -> bool {
    matches!(
        c,
        Configurator::FieldMessage
            | Configurator::BattleSpeed
            | Configurator::SoundVolume
            | Configurator::MusicVolume
            | Configurator::MovieVolume
            | Configurator::VoiceVolume
            | Configurator::AtbMode
    )
}

/// Rows with two choices (value 0 or 1): every non-slider, non-button row
/// (`ConfigUI.cs:1020-1024` and `OnKeyChoice`, `:996-1005`).
pub fn is_choice(c: Configurator) -> bool {
    !is_slider(c)
        && !matches!(
            c,
            Configurator::ControlTutorial
                | Configurator::CombatTutorial
                | Configurator::Title
                | Configurator::QuitGame
        )
}

/// Whether the row exists in the list. `Awake` destroys Sound, SoundEffect
/// and, when a forced message speed is set, FieldMessage (`ConfigUI.cs:1538`);
/// it only creates VoiceVolume and AutoText when voice acting is enabled
/// (`:1523-1527`). The prefab order of the remaining rows is not in source.
pub fn row_present(c: Configurator, voice_acting_enabled: bool, force_message_speed: i32) -> bool {
    match c {
        Configurator::Sound | Configurator::SoundEffect => false,
        Configurator::FieldMessage => force_message_speed < 0,
        Configurator::VoiceVolume | Configurator::AutoText => voice_acting_enabled,
        _ => true,
    }
}

/// `Math.Round(f32 -> f64)`, banker's rounding like C# `Math.Round(double)`.
fn round_even(v: f32) -> f64 {
    f64::from(v).round_ties_even()
}

/// `Mathf.Clamp(value, 0, 1)` (`ConfigUI.cs:1284`).
fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// A volume slider position to its stored percent:
/// `(Int32)Math.Round(value * 20) * 5` (`ConfigUI.cs:1372`).
pub fn volume_from_slider(value: f32) -> i32 {
    (round_even(clamp01(value) * 20.0) as i32) * 5
}

/// Stored percent to slider position (`ConfigUI.cs:1216`: `volume / 100f`).
pub fn slider_from_volume(percent: i32) -> f32 {
    percent as f32 / 100.0
}

/// Slider position of a stored `fld_msg` / `btl_speed` (`ConfigUI.cs:1201,1204`).
pub fn slider_from_cfg(value: u64, step: f32) -> f32 {
    value as f32 / step
}

/// Stored value of a `fld_msg` / `btl_speed` slider position
/// (`ConfigUI.cs:1347,1350`: `(UInt64)Math.Round(value * step)`).
pub fn cfg_from_slider(value: f32, step: f32) -> u64 {
    round_even(clamp01(value) * step) as u64
}

/// ATB mode slider position from the ini value (`ConfigUI.cs:1228-1229`):
/// `min(mode, 3) / 3`.
pub fn slider_from_atb_mode(mode: i32) -> f32 {
    let m = if mode >= 3 { 3 } else { mode };
    m as f32 / 3.0
}

/// Ini ATB mode from the slider position (`ConfigUI.cs:1391-1392`): the
/// truncated `value * 3`, and 5 when that reaches 3.
pub fn atb_mode_from_slider(value: f32) -> i32 {
    let mode = (clamp01(value) * 3.0) as i32;
    if mode >= 3 { 5 } else { mode }
}

/// Slider step of the rows whose step count is in code: volumes use 21
/// steps (`ConfigUI.cs:1489`, so 1/20), the ATB mode 4 steps
/// (`:1507`, so 1/3), per `OnKeyChoice` (`:991-994`: `1 / (numberOfSteps - 1)`).
/// The field-message and battle-speed step counts are prefab data: `None`.
pub fn slider_step(c: Configurator) -> Option<f32> {
    match c {
        Configurator::SoundVolume
        | Configurator::MusicVolume
        | Configurator::MovieVolume
        | Configurator::VoiceVolume => Some(1.0 / 20.0),
        Configurator::AtbMode => Some(1.0 / 3.0),
        _ => None,
    }
}

/// One arrow press on a slider row: `value +- step`, clamped to 0..1.
pub fn nudge_slider(value: f32, step: f32, right: bool) -> f32 {
    clamp01(if right { value + step } else { value - step })
}

/// Both stores the Config screen writes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub cfg: Cfg,
    pub ini: IniOptions,
}

impl Settings {
    /// Current UI value of a row (`DisplayConfigValue`, `ConfigUI.cs:1172-1233`):
    /// 0 or 1 for choices, 0..1 for sliders. `None` for button rows.
    pub fn ui_value(&self, c: Configurator) -> Option<f32> {
        let cfg = &self.cfg;
        let v = |x: u64| Some(x as f32);
        match c {
            Configurator::Sound => v(cfg.sound),
            Configurator::SoundEffect => v(cfg.sound_effect),
            Configurator::Controller => v(cfg.control),
            Configurator::Cursor => v(cfg.cursor),
            Configurator::Atb => v(cfg.atb),
            Configurator::BattleCamera => v(cfg.camera),
            Configurator::SkipBattleCamera => v(cfg.skip_btl_camera),
            Configurator::Movement => v(cfg.move_),
            Configurator::FieldMessage => Some(slider_from_cfg(cfg.fld_msg, FIELD_MESSAGE_STEP)),
            Configurator::BattleSpeed => Some(slider_from_cfg(cfg.btl_speed, BATTLE_SPEED_STEP)),
            Configurator::HereIcon => v(cfg.here_icon),
            Configurator::WindowColor => v(cfg.win_type),
            Configurator::Vibration => v(cfg.vibe),
            Configurator::SoundVolume => Some(slider_from_volume(self.ini.sound_volume)),
            Configurator::MusicVolume => Some(slider_from_volume(self.ini.music_volume)),
            Configurator::MovieVolume => Some(slider_from_volume(self.ini.movie_volume)),
            Configurator::VoiceVolume => Some(slider_from_volume(self.ini.voice_volume)),
            Configurator::AtbMode => Some(slider_from_atb_mode(self.ini.atb_mode)),
            Configurator::AutoText => Some(if self.ini.auto_dismiss_dialog {
                0.0
            } else {
                1.0
            }),
            Configurator::ControlTutorial
            | Configurator::CombatTutorial
            | Configurator::Title
            | Configurator::QuitGame => None,
        }
    }

    /// Writes a row from its UI value (`setConfigValue`, `ConfigUI.cs:1313-1398`).
    /// Choice rows take 0 or 1 (a value that is not exactly 0 or 1 is
    /// truncated like the `(UInt64)` cast); slider rows take 0..1 and are
    /// clamped first. Button rows do nothing.
    ///
    /// The volume rows also zero `cfg.sound_effect` / `cfg.sound`
    /// ("Useless setting now", `ConfigUI.cs:1371,1377`). The source's
    /// `SoundEffect == 1` and `Vibration == ON` branches only start sounds or
    /// a rumble test; they change no stored value.
    pub fn set_ui_value(&mut self, c: Configurator, value: f32) {
        let cfg = &mut self.cfg;
        let raw = value as u64;
        match c {
            Configurator::Sound => cfg.sound = raw,
            Configurator::SoundEffect => cfg.sound_effect = raw,
            Configurator::Controller => cfg.control = raw,
            Configurator::Cursor => cfg.cursor = raw,
            Configurator::Atb => cfg.atb = raw,
            Configurator::BattleCamera => cfg.camera = raw,
            Configurator::SkipBattleCamera => cfg.skip_btl_camera = raw,
            Configurator::Movement => cfg.move_ = raw,
            Configurator::FieldMessage => {
                cfg.fld_msg = cfg_from_slider(value, FIELD_MESSAGE_STEP);
            }
            Configurator::BattleSpeed => {
                cfg.btl_speed = cfg_from_slider(value, BATTLE_SPEED_STEP);
            }
            Configurator::HereIcon => cfg.here_icon = raw,
            Configurator::WindowColor => cfg.win_type = raw,
            Configurator::Vibration => cfg.vibe = raw,
            Configurator::SoundVolume => {
                cfg.sound_effect = 0;
                self.ini.sound_volume = volume_from_slider(value);
            }
            Configurator::MusicVolume => {
                cfg.sound = 0;
                self.ini.music_volume = volume_from_slider(value);
            }
            Configurator::MovieVolume => self.ini.movie_volume = volume_from_slider(value),
            Configurator::VoiceVolume => self.ini.voice_volume = volume_from_slider(value),
            Configurator::AtbMode => self.ini.atb_mode = atb_mode_from_slider(value),
            Configurator::AutoText => self.ini.auto_dismiss_dialog = value == 0.0,
            Configurator::ControlTutorial
            | Configurator::CombatTutorial
            | Configurator::Title
            | Configurator::QuitGame => {}
        }
    }

    /// Left or right arrow on a choice row: `((Int32)value + 1) % 2`
    /// (`ConfigUI.cs:1001`). Returns the new value, `None` for non-choice rows.
    pub fn toggle_choice(&mut self, c: Configurator) -> Option<f32> {
        if !is_choice(c) {
            return None;
        }
        let cur = self.ui_value(c)? as i32;
        let next = (cur + 1) % 2;
        self.set_ui_value(c, next as f32);
        Some(next as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `FF9CFG()` constructor values, `FF9CFG.cs:8-26`.
    #[test]
    fn new_game_defaults() {
        let c = Cfg::default();
        assert_eq!(
            (
                c.sound,
                c.sound_effect,
                c.control,
                c.cursor,
                c.atb,
                c.camera
            ),
            (0, 0, 0, 0, 1, 0)
        );
        assert_eq!(
            (c.move_, c.vibe, c.btl_speed, c.skip_btl_camera),
            (1, 0, 1, 1)
        );
        assert_eq!(
            (c.btl_msg, c.fld_msg, c.here_icon, c.win_type),
            (3, 3, 0, 0)
        );
        assert_eq!(c.control_data_keyboard, [0; 10]);
        assert_eq!(c.control_data_joystick, vec![String::new(); 10]);
        // FF9CFG_VIBE_ON is the zero-initialized static byte (FF9CFG.cs:50).
        assert_eq!(c.vibe, VIBE_ON);
    }

    /// `GetATBCoef`: 8, 10, 14, else 10 (`btl_para.cs:68-75`).
    #[test]
    fn atb_coef_table() {
        let mut c = Cfg::default();
        assert_eq!(c.atb_coef(), 10);
        c.btl_speed = 0;
        assert_eq!(c.atb_coef(), 8);
        c.btl_speed = 2;
        assert_eq!(c.atb_coef(), 14);
        c.btl_speed = 9;
        assert_eq!(c.atb_coef(), 10);
    }

    /// Default fld_msg 3 reads tick 16 (`Dialog.cs:1688-1691`), btl_msg 3
    /// reads 30 (`BattleHUD.Const.cs:34`); both tables end where the slider
    /// ends (`fld_msg` 6 reads index 6, `btl_msg` 6 is the last row).
    #[test]
    fn message_ticks() {
        let c = Cfg::default();
        assert_eq!(c.field_message_tick(), Some(16));
        assert_eq!(c.battle_message_tick(), Some(30));
        let mut c = c;
        c.fld_msg = 6;
        c.btl_msg = 6;
        assert_eq!(c.field_message_tick(), Some(64));
        assert_eq!(c.battle_message_tick(), Some(12));
        c.btl_msg = 7;
        assert_eq!(c.battle_message_tick(), None);
    }

    /// Slider math, `ConfigUI.cs:1201,1204,1347,1350`: fld_msg 3 sits at
    /// 3/6 = 0.5, and 0.5 * 6 rounds back to 3; btl_speed 1 sits at 0.5 and
    /// 0.5 * 2 is 1. Banker's rounding: 0.25 * 6 = 1.5 rounds to 2, and
    /// 0.75 * 6 = 4.5 rounds to 4.
    #[test]
    fn cfg_slider_round_trip() {
        assert_eq!(slider_from_cfg(3, FIELD_MESSAGE_STEP), 0.5);
        assert_eq!(cfg_from_slider(0.5, FIELD_MESSAGE_STEP), 3);
        assert_eq!(cfg_from_slider(0.5, BATTLE_SPEED_STEP), 1);
        assert_eq!(cfg_from_slider(0.25, FIELD_MESSAGE_STEP), 2);
        assert_eq!(cfg_from_slider(0.75, FIELD_MESSAGE_STEP), 4);
        // Mathf.Clamp(value, 0, 1) first: 2.0 acts as 1.0.
        assert_eq!(cfg_from_slider(2.0, FIELD_MESSAGE_STEP), 6);
        assert_eq!(cfg_from_slider(-1.0, BATTLE_SPEED_STEP), 0);
    }

    /// Volume: `(Int32)Math.Round(value * 20) * 5` (`ConfigUI.cs:1372`).
    /// 0.5 * 20 = 10 so 50; 0.07 * 20 = 1.4 rounds to 1 so 5; 1.0 gives 100.
    #[test]
    fn volume_math() {
        assert_eq!(volume_from_slider(0.5), 50);
        assert_eq!(volume_from_slider(0.07), 5);
        assert_eq!(volume_from_slider(1.0), 100);
        assert_eq!(volume_from_slider(0.0), 0);
        assert_eq!(slider_from_volume(100), 1.0);
        assert_eq!(slider_step(Configurator::MusicVolume), Some(0.05));
        assert_eq!(slider_step(Configurator::FieldMessage), None);
    }

    /// ATB mode, `ConfigUI.cs:1228-1229,1391-1392`: ini 0, 1, 2 map to
    /// positions 0, 1/3, 2/3; ini 3, 4, 5 all show the last stop; the last
    /// stop (3 after truncation) is stored as 5.
    #[test]
    fn atb_mode_math() {
        assert_eq!(slider_from_atb_mode(0), 0.0);
        assert_eq!(slider_from_atb_mode(2), 2.0 / 3.0);
        assert_eq!(slider_from_atb_mode(4), 1.0);
        assert_eq!(atb_mode_from_slider(0.0), 0);
        assert_eq!(atb_mode_from_slider(1.0 / 3.0), 1);
        assert_eq!(atb_mode_from_slider(2.0 / 3.0), 2);
        assert_eq!(atb_mode_from_slider(1.0), 5);
        assert_eq!(slider_step(Configurator::AtbMode), Some(1.0 / 3.0));
    }

    /// Choice rows toggle between 0 and 1 (`ConfigUI.cs:1001`).
    #[test]
    fn toggle_and_set() {
        let mut s = Settings::default();
        assert_eq!(s.ui_value(Configurator::Atb), Some(1.0));
        assert_eq!(s.toggle_choice(Configurator::Atb), Some(0.0));
        assert_eq!(s.cfg.atb, 0);
        assert_eq!(s.toggle_choice(Configurator::Atb), Some(1.0));
        assert_eq!(s.cfg.atb, 1);
        assert_eq!(s.toggle_choice(Configurator::Vibration), Some(1.0));
        assert_eq!(s.cfg.vibe, VIBE_OFF);
        // Sliders and buttons do not toggle.
        assert_eq!(s.toggle_choice(Configurator::BattleSpeed), None);
        assert_eq!(s.toggle_choice(Configurator::Title), None);
    }

    /// The volume rows zero the old on/off fields (`ConfigUI.cs:1371,1377`).
    #[test]
    fn volume_rows_clear_legacy_switches() {
        let mut s = Settings::default();
        s.cfg.sound = 1;
        s.cfg.sound_effect = 1;
        s.set_ui_value(Configurator::MusicVolume, 0.25);
        assert_eq!((s.cfg.sound, s.cfg.sound_effect), (0, 1));
        assert_eq!(s.ini.music_volume, 25);
        s.set_ui_value(Configurator::SoundVolume, 0.5);
        assert_eq!(s.cfg.sound_effect, 0);
        assert_eq!(s.ini.sound_volume, 50);
    }

    /// Auto-text: the UI value is 0 when auto dismiss is on (`ConfigUI.cs:1231,1396`).
    #[test]
    fn auto_text_inverted() {
        let mut s = Settings::default();
        assert_eq!(s.ui_value(Configurator::AutoText), Some(1.0));
        s.set_ui_value(Configurator::AutoText, 0.0);
        assert!(s.ini.auto_dismiss_dialog);
        assert_eq!(s.ui_value(Configurator::AutoText), Some(0.0));
    }

    /// Row presence, `ConfigUI.cs:1523-1527,1538`.
    #[test]
    fn rows_present() {
        assert!(!row_present(Configurator::Sound, true, -1));
        assert!(!row_present(Configurator::SoundEffect, true, -1));
        assert!(row_present(Configurator::FieldMessage, false, -1));
        assert!(!row_present(Configurator::FieldMessage, false, 2));
        assert!(!row_present(Configurator::VoiceVolume, false, -1));
        assert!(row_present(Configurator::AutoText, true, -1));
        assert!(row_present(Configurator::Cursor, false, 0));
        assert!(is_slider(Configurator::AtbMode));
        assert!(is_choice(Configurator::HereIcon));
        assert!(!is_choice(Configurator::QuitGame));
    }

    /// Defaults survive the save tree: the saved block has no `btl_msg`.
    #[test]
    fn save_section_round_trip() {
        let mut tree = crate::saves::ff9_tree::Ff9SaveTree::blank();
        let mut cfg = Cfg {
            cursor: 1,
            fld_msg: 5,
            btl_msg: 0, // not saved
            ..Cfg::default()
        };
        cfg.control_data_keyboard[2] = 97;
        cfg.control_data_joystick[1] = "JoystickButton3".to_string();
        cfg.write_setting_section(&mut tree.setting);
        let back = Cfg::from_setting_section(&tree.setting);
        assert_eq!(back.cursor, 1);
        assert_eq!(back.fld_msg, 5);
        assert_eq!(back.btl_msg, 3);
        assert_eq!(back.control_data_keyboard[2], 97);
        assert_eq!(back.control_data_joystick[1], "JoystickButton3");
        assert_eq!(back.atb, 1);
    }
}
