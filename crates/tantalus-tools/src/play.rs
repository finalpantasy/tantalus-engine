//! A playable run: new game from field 70 on, driven by pad input.
//!
//! [`PlayGame`] owns the world between fields and the live
//! [`FieldSession`] of the current one. Each [`PlayGame::tick`] takes the
//! held `EventInput` bits, walks the player on the walkmesh, closes dialog
//! windows on a confirm press (with the choice cursor for choice windows),
//! fights the field's battles with [`FieldBattles`], and loads the next
//! field when the scripts change to one. Nothing here draws; the client's
//! `play` window and the trace's `play` command both sit on top of it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use tantalus_contracts::Point3;
use tantalus_game::field::FieldWorld;
use tantalus_game::new_game::BaseStats;

use crate::field_battle::{
    FieldBattles, FightControl, LiveFight, PartyFighter, PartyGlobals, pay_rewards, save_party,
};
use crate::field_data::{
    FieldText, load_cameras, load_navigation, load_tile_animations, parse_background_map,
    parse_event_db,
};
use crate::field_menu::{FieldMenu, MenuAction, SlotFile, SlotRow};
use crate::field_session::{
    CONFIRM, FieldSession, WorldExit, decode_pool, load_event, prepare_field_world, world_exit,
};
use crate::play_audio::{AudioCue, menu_sound};
use crate::play_save::{self, PlaySave, SAVE_FORMAT, SLOTS};
use crate::shop_menu::ShopMenu;
use crate::steam_save::{self, Resume, SteamSave};
use crate::world_data::WorldAssets;
use crate::world_play::WorldPlay;
use tantalus_game::battle::BattleCommand;
use tantalus_legacy::HostEvent;

/// `EventInput` pad bits (Memoria `Global/Event/EventInput.cs`).
/// The script's bit for the Special control (`Control.Special`).
const SPECIAL_KEY: i32 = 0x80000;

/// Field ticks a second (the play time counts them).
pub(crate) const TICKS_PER_SECOND: u64 = 30;

pub mod pad {
    pub const SELECT: i32 = 0x1;
    pub const START: i32 = 0x8;
    pub const UP: i32 = 0x10;
    pub const RIGHT: i32 = 0x20;
    pub const DOWN: i32 = 0x40;
    pub const LEFT: i32 = 0x80;
    pub const L2: i32 = 0x100;
    pub const R2: i32 = 0x200;
    pub const L1: i32 = 0x400;
    pub const R1: i32 = 0x800;
    pub const TRIANGLE: i32 = 0x1000;
    pub const CIRCLE: i32 = 0x2000;
    pub const CROSS: i32 = 0x4000;
    pub const SQUARE: i32 = 0x8000;
    pub const CANCEL: i32 = 0x10000;
    pub const CONFIRM: i32 = super::CONFIRM;
}

/// Button prompts that close themselves once their key is pressed, as
/// (map, text id, key): pressing confirm leaves these to their script.
pub const PROMPT_KEYS: [(i32, u16, i32); 9] = [
    (206, 352, 0x1),
    (64, 112, 0x80),
    (64, 113, 0x20),
    (64, 114, 0x1000),
    (64, 115, 0x40),
    (64, 116, 0x4000),
    (64, 117, 0x10),
    (64, 118, 0x2000),
    (64, 119, 0x8000),
];

/// Where the install and the Memoria checkout are.
#[derive(Debug, Clone)]
pub struct PlayPaths {
    /// `FINAL FANTASY IX/StreamingAssets`.
    pub streaming_assets: PathBuf,
    /// `FINAL FANTASY IX/x64/FF9_Data`.
    pub game_data: PathBuf,
    /// Memoria source root (event DB, text map, character tables).
    pub memoria: PathBuf,
    /// `ratan_tbl.bin`.
    pub atan_table: PathBuf,
    /// `audit/battle-scene-ids.tsv`; without it battles are skipped as won.
    pub battles: Option<PathBuf>,
    pub language: String,
}

/// Everything loaded once for a run.
pub struct PlayData {
    streaming: PathBuf,
    bundle: PathBuf,
    field_bundles: Vec<PathBuf>,
    db: BTreeMap<i32, String>,
    backgrounds: BTreeMap<i32, String>,
    text: FieldText,
    item_names: BTreeMap<i32, String>,
    /// Field location names by field id (`FF9TextTool.LocationName`, `loc_name.mes`).
    location_names: BTreeMap<i32, String>,
    /// World map area names by area (`FF9TextTool.WorldLocationText`, `worldloc.mes`).
    world_locations: BTreeMap<i32, String>,
    game_data: PathBuf,
    battles: Option<FieldBattles>,
    battle_base: BTreeMap<i32, BaseStats>,
    new_game: FieldWorld,
    menu: crate::field_menu::MenuTables,
    shops: crate::shop_menu::ShopTables,
    streaming_assets: PathBuf,
    /// The translated plugin that times effects; its sound commands drive the spell sounds.
    plugin: Option<std::sync::Arc<crate::battle_effects::PluginTiming>>,
    /// `Actions.csv`, for the ability to effect map of the spell sounds.
    actions_csv: PathBuf,
    /// The spell and ability sounds, read from the game files on the first battle.
    effect_sounds: std::sync::OnceLock<Option<crate::battle_sound::EffectSounds>>,
    /// `MBG.MBGDiscTable`: `(disc, fmv)` to movie name; empty without `MBG.cs`.
    movies: BTreeMap<(i32, i32), String>,
    /// The Tetra Master tables; `None` when the install's `resources.assets` has none.
    cards: Option<std::sync::Arc<tantalus_game::cards::CardTables>>,
    /// Memoria's `EmbeddedAsset/UI/Sprites` folder (the disc-change pictures).
    sprites: PathBuf,
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

impl PlayData {
    pub fn load(paths: &PlayPaths) -> Result<Self, String> {
        let sa = &paths.streaming_assets;
        let memoria = &paths.memoria;
        // The event scripts follow the text tables' language (`Localization.CurrentSymbol`).
        crate::field_session::set_asset_language(&paths.language);
        let bundle = sa.join("p0data7.bin");
        let field_bundles: Vec<PathBuf> = (11..=19)
            .map(|n| sa.join(format!("p0data{n}.bin")))
            .collect();
        let db = parse_event_db(&read_text(
            &memoria.join("Assembly-CSharp/Global/ff9/FF9DBAll.Events.cs"),
        )?);
        let mes_source =
            read_text(&memoria.join("Assembly-CSharp/Global/Event/Engine/EventEngineUtils.cs"))?;
        let backgrounds = parse_background_map(&mes_source);
        let text = FieldText::open(&paths.game_data, &mes_source, paths.language.clone())?;
        let item_names = text.item_names().unwrap_or_default();
        let location_names = text.location_names().unwrap_or_default();
        let world_locations = text.names("etc/worldloc").unwrap_or_default();
        let characters = memoria.join("Memoria.Patcher/StreamingAssets/Data/Characters");
        let base = tantalus_game::new_game::parse_base_stats(&read_text(
            &characters.join("BaseStats.csv"),
        )?)?;
        let levels =
            tantalus_game::new_game::parse_leveling(&read_text(&characters.join("Leveling.csv"))?)?;
        let atan = std::fs::read(&paths.atan_table)
            .map_err(|error| format!("cannot read {}: {error}", paths.atan_table.display()))?;
        let mut new_game = new_game_world(&base, &levels, atan);
        {
            // The frame count of each field's SPS files, for the effects scripts load.
            let library = std::sync::Arc::new(
                tantalus_assets::pc::field_sps::FieldSpsLibrary::new(field_bundles.clone()),
            );
            let names = backgrounds.clone();
            new_game.sps_frame_counts = Some(std::sync::Arc::new(move |field, id| {
                let sps = library.field(names.get(&field)?)?;
                Some(sps.files.get(&id)?.frame_count)
            }));
        }
        let mut menu = crate::field_menu::MenuTables::load(
            memoria,
            base.clone(),
            levels.iter().map(|row| row.experience).collect(),
            item_names.clone(),
            text.names("ability/aa_name").unwrap_or_default(),
            text.names("ability/sa_name").unwrap_or_default(),
        )?;
        menu.card_names = text.names("etc/minista").unwrap_or_default();
        menu.card_levels = text.names("etc/card").unwrap_or_default();
        menu.chocograph_text = text.names("etc/ff9choco").unwrap_or_default();
        menu.item_help = text.menu_text("item/itm_help").unwrap_or_default();
        menu.key_names = text.names("keyitem/imp_name").unwrap_or_default();
        menu.key_help = text.menu_text("keyitem/imp_help").unwrap_or_default();
        menu.key_skin = text.menu_text("keyitem/imp_skin").unwrap_or_default();
        menu.seed_equipment(&mut new_game);
        // `FF9Play_SetDefEquips`' sets for `JOIN`; the tables carry non-UTF-8 comments.
        let lossy = |name: &str| {
            let path = characters.join(name);
            std::fs::read(&path)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .map_err(|error| format!("cannot read {}: {error}", path.display()))
        };
        new_game.equipment_sets = Some(std::sync::Arc::new(
            tantalus_game::new_game::parse_equipment_sets(
                &lossy("DefaultEquipment.csv")?,
                &lossy("CharacterParameters.csv")?,
            )?,
        ));
        // `JOIN`'s `FF9FEqp_UpdateSA` reads the item and ability tables.
        new_game.support_refresh = Some(crate::ability_menu::support_refresh(std::sync::Arc::new(
            menu.clone(),
        )));
        let shops = crate::shop_menu::ShopTables::load(memoria)?;
        new_game.items = tantalus_game::new_game::parse_initial_items(&read_text(
            &characters.join("../Items/InitialItems.csv"),
        )?)?;
        let mut plugin = None;
        let battles = match &paths.battles {
            Some(ids) => Some(FieldBattles::load(sa, &bundle, ids, {
                let mut equipment = crate::field_battle::Equipment::load(&characters)?;
                equipment.load_plugin_timing(&paths.game_data);
                plugin = equipment.plugin();
                equipment
            })?),
            None => None,
        };
        Ok(Self {
            streaming: sa.clone(),
            bundle,
            field_bundles,
            db,
            backgrounds,
            text,
            item_names,
            location_names,
            world_locations,
            game_data: paths.game_data.clone(),
            battles,
            battle_base: base,
            new_game,
            menu,
            shops,
            streaming_assets: sa.clone(),
            plugin,
            actions_csv: memoria.join("Memoria.Patcher/StreamingAssets/Data/Battle/Actions.csv"),
            effect_sounds: std::sync::OnceLock::new(),
            movies: std::fs::read_to_string(memoria.join("Assembly-CSharp/Global/MBG.cs"))
                .map(|source| crate::movie::parse_disc_table(&source))
                .unwrap_or_default(),
            cards: crate::card_play::load_tables(&paths.game_data)
                .ok()
                .map(std::sync::Arc::new),
            sprites: memoria.join("Memoria.Patcher/FF9_Data/EmbeddedAsset/UI/Sprites"),
        })
    }

    /// Every field map number the event DB names, in order.
    pub fn field_ids(&self) -> Vec<i32> {
        self.db.keys().copied().collect()
    }

    /// The field's parsed event binary, when the DB names it and the bundle has it.
    pub fn field_binary(&self, map: i32) -> Option<tantalus_legacy::EventBinary> {
        let name = self.db.get(&map)?;
        load_event(&self.bundle, &format!("{name}.eb")).ok()
    }

    /// Starts field `map` on the new-game world the way entering it does:
    /// event pool, walkmesh and dialog signals. `Ok(None)` when the map has
    /// no event binary.
    pub fn start_field(&self, map: i32) -> Result<Option<FieldSession>, String> {
        let Some(binary) = self.field_binary(map) else {
            return Ok(None);
        };
        let (pool, _) = decode_pool(&binary)?;
        let mut world = self.new_game.clone();
        prepare_field_world(&mut world, map);
        if let Some(Ok(cameras)) = self
            .backgrounds
            .get(&map)
            .map(|background| load_cameras(&self.field_bundles, background))
        {
            world.provide_cameras(cameras);
        }
        match self.backgrounds.get(&map) {
            Some(background) => match load_navigation(&self.field_bundles, background) {
                Ok(mesh) => world.provide_navigation(mesh),
                Err(_) => world.clear_navigation(),
            },
            None => world.clear_navigation(),
        }
        if let Ok(info) = self.text.info(map) {
            world.dialog_signals = info.signals;
        }
        FieldSession::start(map, world, pool).map(Some)
    }

    pub fn field_name(&self, map: i32) -> Option<&str> {
        self.db.get(&map).map(String::as_str)
    }

    /// The place name the save screen shows for field `map` (`FF9TextTool.LocationName`,
    /// which `EventEngine` copies to `FF9.mapNameStr` on entering the field).
    pub fn place_name(&self, map: i32) -> Option<&str> {
        self.location_names
            .get(&map)
            .map(String::as_str)
            .filter(|name| !name.is_empty())
    }

    /// The world map area name of `area` (`FF9TextTool.WorldLocationText`, which
    /// `UIManager.MenuOpenEvent` copies to `FF9.mapNameStr` from system variable 192).
    pub fn world_place_name(&self, area: i32) -> Option<&str> {
        self.world_locations
            .get(&area)
            .map(String::as_str)
            .filter(|name| !name.is_empty())
    }

    pub fn has_battles(&self) -> bool {
        self.battles.is_some()
    }
}

/// `FF9Play_Init`: every character's record at the new-game level, the
/// level-up curve for EXP, and the atan table the walk code reads.
pub fn new_game_world(
    base: &BTreeMap<i32, BaseStats>,
    levels: &[tantalus_game::new_game::LevelRow],
    atan: Vec<u8>,
) -> FieldWorld {
    let mut world = FieldWorld::new();
    world.character_stats = tantalus_game::new_game::new_game_character_stats(base, levels);
    let (base, levels) = (base.clone(), levels.to_vec());
    world.level_experience = Some(std::sync::Arc::new(
        levels.iter().map(|row| u64::from(row.experience)).collect(),
    ));
    world.level_growth = Some(std::sync::Arc::new(move |character, total_exp| {
        let base = base.get(&character)?;
        let level = levels
            .iter()
            .enumerate()
            .filter(|(_, row)| u64::from(row.experience) <= total_exp)
            .map(|(index, _)| index + 1)
            .max()
            .unwrap_or(1)
            .min(levels.len());
        tantalus_game::new_game::build_stats(*base, &levels, level)
    }));
    world.provide_atan_table(atan);
    crate::animation_frames::install(&mut world);
    world
}

/// What happened during one [`PlayGame::tick`].
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum PlayEvent {
    /// A field started.
    Field {
        map: i32,
        name: String,
        navigation: String,
    },
    /// A window opened with this text.
    Dialog {
        window: u8,
        text: u16,
        line: String,
        choice: bool,
    },
    /// A battle was fought (or skipped without battle data).
    Battle { battle: i32, outcome: String },
    /// The world map led to a field (or nowhere).
    WorldMap {
        world_map: i32,
        location: Option<i32>,
        next: Option<i32>,
    },
    /// A talk (tag 3) or region (tag 2) function started.
    Script { entry: usize, tag: u16 },
    /// The player walks the world map from `place`.
    WorldWalk { place: i32 },
    /// The script changed disc (`DISCCHANGE`): `disc` (0-based) starts at `field` once the
    /// disc picture and the save prompt are done.
    DiscChange { field: i32, disc: u8 },
    /// A field script ended the game (`GAMEOVER`): the game over screen, then the title.
    GameOver,
    /// The run cannot go on.
    Stopped(String),
}

/// One open dialog window, ready to draw.
#[derive(Debug, Clone)]
pub struct OpenDialog {
    pub window: u8,
    pub text: u16,
    /// The text entry with names, numbers and items filled in.
    pub entry: tantalus_assets::pc::mes::MesEntry,
    pub layout: crate::field_data::TextLayout,
    /// Script window flags (`MES` argument 2).
    pub flags: u8,
    /// The actor the window follows when its flags say chat style.
    pub speaker: Option<tantalus_contracts::EntityId>,
    pub choosing: bool,
}

/// `CharacterId` of a character-name keyword tag (`[ZDNE]`, ...).
fn keyword_character(keyword: &str) -> i32 {
    match keyword {
        "ZDNE" => 0,
        "VIVI" => 1,
        "DGGR" => 2,
        "STNR" => 3,
        "FRYA" => 4,
        "QUIN" => 5,
        "EIKO" => 6,
        "AMRT" => 7,
        _ => 0,
    }
}

/// Default name of a `CharacterId` (`CharacterParameters.csv`; the game has
/// no rename screen here).
pub fn character_name(id: i32) -> &'static str {
    match id {
        1 => "Vivi",
        2 => "Dagger",
        3 => "Steiner",
        4 => "Freya",
        5 => "Quina",
        6 => "Eiko",
        7 => "Amarant",
        // `FF9TextTool.CharacterDefaultName` for the guest cast.
        8 => "Cinna",
        9 => "Marcus",
        10 => "Blank",
        11 => "Beatrix",
        _ => "Zidane",
    }
}

/// The run.
pub struct PlayGame {
    data: PlayData,
    session: Option<FieldSession>,
    world: FieldWorld,
    choice_texts: BTreeSet<u16>,
    plain: BTreeMap<u16, String>,
    layouts: BTreeMap<u16, crate::field_data::TextLayout>,
    entries: BTreeMap<u16, tantalus_assets::pc::mes::MesEntry>,
    shown: BTreeMap<u8, u16>,
    previous: i32,
    /// Answer highlighted in an open choice window.
    pub cursor: i32,
    /// World map locations to leave through, used in order.
    pub world_exits: Vec<i32>,
    /// Walk the world map when a field leaves for it and no scripted
    /// `world_exits` remain; off, the run stops there as before.
    pub world_mode: bool,
    /// The overworld while the player walks it.
    pub world_play: Option<WorldPlay>,
    /// The Chocograph list the world script opened (`ChocographUI`).
    pub chocograph_screen: Option<crate::chocograph_menu::ChocographScreen>,
    /// The world map whose script runs while walking (9000 to 9012, the
    /// number the field that left for the world asked for).
    pub world_script_map: i32,
    world_assets: Option<std::sync::Arc<WorldAssets>>,
    world_exit: Option<WorldExit>,
    pub stopped: Option<String>,
    pub ticks: u64,
    /// Walkmesh floor the player stands on (found on the first step).
    floor: Option<i16>,
    /// The last battle fought, with its stage record, for a client to
    /// replay ([`crate::battle_stage`]). Take it after a
    /// [`PlayEvent::Battle`].
    pub last_battle: Option<crate::battle_stage::BattleStage>,
    /// What the last live battle paid, for the result screen. Take it
    /// after a [`PlayEvent::Battle`].
    pub last_results: Option<BattleResults>,
    /// The open main menu; the field does not run while it is open.
    menu: Option<FieldMenu>,
    /// The open shop; the field does not run while it is open.
    shop: Option<ShopMenu>,
    /// The naming screen a `MENU(1, character)` opened.
    name_screen: Option<crate::name_menu::NameMenu>,
    /// The cursor on the party screen a field script opened (`PARTYMENU`).
    party_select: crate::party_select::PartySelect,
    save_dir: PathBuf,
    /// Where to put the player after a load, once their actor exists.
    pending_position: Option<[f32; 3]>,
    /// Fight battles one battle tick at a time with [`Self::battle_tick`]
    /// (a player's window) instead of to the end inside the field tick
    /// that starts them. Off by default: the headless paths are unchanged.
    pub live_battles: bool,
    /// Hand movies to a player's window ([`Self::movie`]) and hold the field while one plays.
    /// Off by default: the headless paths are unchanged.
    pub live_movies: bool,
    sound_camera: Option<tantalus_contracts::LegacyFieldCamera>,
    battle_sounds: crate::battle_sound::BattleSounds,
    /// The volumes last handed to the audio, so a change is sent once.
    sent_volumes: Option<(i32, i32)>,
    /// The battles a field tick started, while [`Self::live_battles`]
    /// fights them; the field waits until they are over.
    pending: Option<PendingBattles>,
    /// Sound requests made since the last [`PlayGame::take_audio`], with the tick of each.
    /// Runtime-only: the run never reads them back.
    audio: Vec<(u64, AudioCue)>,
    /// The Tetra Master game being played; the field waits until it ends.
    card: Option<tantalus_game::cards::CardGame>,
    /// The Steam save file the game came from; when set the save and load screens use it
    /// instead of the JSON slots.
    steam: Option<SteamFile>,
    /// A disc change in progress (`DISCCHANGE`); the field waits until it is done.
    disc_change: Option<DiscChange>,
    /// The game over screen a field script's `GAMEOVER` opened (`GameOverUI`).
    game_over: Option<GameOverScreen>,
    /// The title screen while it is up, and the field a new game starts on.
    title: Option<(crate::title_menu::TitleScreen, i32)>,
    /// The title is the PS1's (NEW GAME and CONTINUE from the disc's art), not `TitleUI`'s.
    ps1_title: bool,
    /// The ride whose world song was last asked for (`w_musicRequestNoIdx`); `None` off the
    /// world map. Runtime-only.
    world_song_ride: Option<u8>,
    /// The tick a field started on, until its `FieldReady` cue is sent. Runtime-only.
    field_ready_due: Option<u64>,
    /// A live battle ended and its result screen is up: the client sends `BattleLeft` when it
    /// closes ([`PlayGame::battle_result_closed`]). Runtime-only.
    result_screen_open: bool,
}

/// `ff9.w_musicChoice` (`ff9.cs:6468`) over `ff9.w_musicSet`: the world song of the ride's music
/// index (`s_moveCHRControl.music`) at story point `scenario` (`gEventGlobal` word 0). On foot,
/// the Mist continent's late story and the desert get their own songs, and disc 4 (scenario
/// 11090 and up, `WorldConfiguration.GetDisc`) another.
pub fn world_song(music: u8, scenario: i32) -> i32 {
    const SET: [i32; 8] = [69, 100, 112, 45, 95, 96, 61, 62];
    let mut index = if usize::from(music) >= SET.len() {
        0
    } else {
        usize::from(music)
    };
    if index == 0 {
        if (9600..9890).contains(&scenario) {
            index = 4;
        }
        if (9890..9910).contains(&scenario) {
            index = 5;
        }
        if scenario >= 11090 {
            index = 3;
        }
    }
    SET[index]
}

/// Ticks the disc picture takes to fade in or out: `SceneDirector` draws it over 100 frames
/// each way, at 60 frames a second; the play ticks at 30.
/// `GameOverUI.duration`: the picture fades in and out over 1 second (30 ticks).
pub const GAME_OVER_FADE_TICKS: u32 = 30;

/// The game over screen of a field script's `GAMEOVER` (`HonoluluFieldMain` `FLOW_STATE_GAMEOVER`
/// -> `EventService.OpenGameOver` -> `GameOverUI`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOverScreen {
    pub stage: GameOverStage,
    /// Ticks in the stage.
    pub ticks: u32,
}

/// Where the game over screen is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameOverStage {
    /// `Show`: `AnimatePanel(1)`, the picture fades in.
    FadeIn,
    /// Up until Confirm (`OnKeyConfirm` -> `GoToTitleScene`).
    Wait,
    /// `AnimatePanel(0)`, then `WaitAndHide` -> `Hide` -> the title.
    FadeOut,
}

impl GameOverScreen {
    /// The picture's opacity.
    pub fn alpha(&self) -> f32 {
        let t = (self.ticks as f32 / GAME_OVER_FADE_TICKS as f32).min(1.0);
        match self.stage {
            GameOverStage::FadeIn => t,
            GameOverStage::Wait => 1.0,
            GameOverStage::FadeOut => 1.0 - t,
        }
    }
}

pub const DISC_FADE_TICKS: u32 = 50;

/// The end of a disc (`EventEngine.DoEventCode.cs` `case DISCCHANGE`): the script sets
/// `FieldMapNo` and asks for a scene change, `HonoluluFieldMain.ff9fieldDiscCondition` loads
/// that field, and `SceneDirector.InitDiscChange` (with `DisplayPSXDiscChanges`, on by
/// default) shows `changetodisc<N>.png` until a button is pressed. Then the save screen
/// opens, as the PSX does between discs, and the next disc starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscChange {
    /// The first field of the next disc.
    pub field: i32,
    /// The next disc, 0-based (`fieldAndDiscDest >> 14 & 3`).
    pub disc: u8,
    pub stage: DiscStage,
    /// Ticks spent in the current stage.
    pub ticks: u32,
}

/// Where a [`DiscChange`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscStage {
    /// The disc picture fades in and waits for a press.
    Picture,
    /// The disc picture fades out.
    FadeOut,
    /// The save screen is open.
    Save,
}

impl DiscChange {
    /// The picture's disc number, 2 to 4, while it shows (`SceneDirector._discChange + 1`;
    /// only discs 1 to 3 have one).
    pub fn picture(&self) -> Option<u8> {
        (matches!(self.stage, DiscStage::Picture | DiscStage::FadeOut)
            && (1..4).contains(&self.disc))
        .then_some(self.disc + 1)
    }

    /// How opaque the picture is, 0 to 1.
    pub fn alpha(&self) -> f32 {
        let t = (self.ticks as f32 / DISC_FADE_TICKS as f32).min(1.0);
        match self.stage {
            DiscStage::Picture => t,
            DiscStage::FadeOut => 1.0 - t,
            DiscStage::Save => 0.0,
        }
    }
}

/// A Steam save file (`SavedData_ww.dat`) the play window loaded from and saves to.
#[derive(Debug, Clone)]
pub struct SteamFile {
    /// The file the game was loaded from. Never written.
    pub source: PathBuf,
    /// The file saves go to: a copy of `source` made on the first save, so the original
    /// file stays as it was.
    pub output: PathBuf,
    /// The file's slot (0 to 9) whose save positions the save and load screens show.
    pub slot: i32,
    /// The state the game came from; sections Tantalus does not model are saved from it.
    pub base: tantalus_legacy::saves::state::SaveState,
}

/// Battles a field tick started, fought live, and the rest of that tick.
struct PendingBattles {
    requests: Vec<tantalus_game::field::EncounterRequest>,
    /// The fight of `requests[0]`.
    fight: Option<LiveFight>,
    next_field: Option<i32>,
    world_map: Option<i32>,
    /// The last stage, for [`PlayGame::last_battle`].
    stage: Option<crate::battle_stage::BattleStage>,
}

impl PlayGame {
    /// New game, starting at field `start` (70 is the opening).
    pub fn new(data: PlayData, start: i32) -> Result<(Self, Vec<PlayEvent>), String> {
        Self::new_with_globals(data, start, &[])
    }

    /// [`PlayGame::new`] with script globals written before the field's scripts run, to start a
    /// probe at a chosen story state (a save-like state): `(index, kind, value)`.
    pub fn new_with_globals(
        data: PlayData,
        start: i32,
        globals: &[(u16, tantalus_legacy::VariableType, i32)],
    ) -> Result<(Self, Vec<PlayEvent>), String> {
        let mut world = data.new_game.clone();
        for &(index, kind, value) in globals {
            world.global_memory.set(index, kind, value);
        }
        let mut game = Self {
            data,
            session: None,
            world,
            choice_texts: BTreeSet::new(),
            plain: BTreeMap::new(),
            layouts: BTreeMap::new(),
            entries: BTreeMap::new(),
            shown: BTreeMap::new(),
            previous: 0,
            cursor: 0,
            world_exits: Vec::new(),
            world_mode: false,
            world_play: None,
            chocograph_screen: None,
            world_script_map: 9000,
            world_assets: None,
            world_exit: None,
            stopped: None,
            ticks: 0,
            floor: None,
            last_battle: None,
            last_results: None,
            menu: None,
            shop: None,
            name_screen: None,
            party_select: Default::default(),
            save_dir: play_save::default_dir(),
            pending_position: None,
            live_battles: false,
            live_movies: false,
            sound_camera: None,
            battle_sounds: crate::battle_sound::BattleSounds::default(),
            sent_volumes: None,
            pending: None,
            audio: Vec::new(),
            card: None,
            steam: None,
            disc_change: None,
            game_over: None,
            title: None,
            ps1_title: false,
            world_song_ride: None,
            field_ready_due: None,
            result_screen_open: false,
        };
        let mut events = Vec::new();
        game.enter(start, &mut events)?;
        Ok((game, events))
    }

    /// The install's `FF9_Data` folder (font and window art live there).
    pub fn game_data(&self) -> &Path {
        &self.data.game_data
    }

    pub fn map(&self) -> Option<i32> {
        self.session.as_ref().map(|session| session.map)
    }

    pub fn field_name(&self) -> Option<&str> {
        self.data.field_name(self.map()?)
    }

    /// Background (`FBG_...`) of the current field, which names its
    /// render bundle entry.
    pub fn background(&self) -> Option<&str> {
        self.data.backgrounds.get(&self.map()?).map(String::as_str)
    }

    /// Background (`FBG_...`) of field `map`, for effects that read another field's files.
    pub fn background_of(&self, map: i32) -> Option<&str> {
        self.data.backgrounds.get(&map).map(String::as_str)
    }

    /// The live world (the current field's, or the one between fields).
    pub fn world(&self) -> &FieldWorld {
        match &self.session {
            Some(session) => session.director.world(),
            None => &self.world,
        }
    }

    /// The running field's scripts, for probes and tests.
    pub fn session(&self) -> Option<&FieldSession> {
        self.session.as_ref()
    }

    /// Text of each open window, in window order.
    pub fn open_text(&self) -> Vec<(u8, String)> {
        let world = self.world();
        world
            .windows
            .iter()
            .filter(|(_, open)| **open)
            .filter_map(|(window, _)| {
                let text = world.window_texts.get(window)?;
                Some((*window, self.plain.get(text).cloned().unwrap_or_default()))
            })
            .collect()
    }

    /// Each open window, in window order, with what a renderer needs: the
    /// text entry with names, numbers and items filled in, its layout hints,
    /// and the script flags and speaker. Windows whose text failed to load
    /// are left out.
    pub fn open_dialogs(&self) -> Vec<OpenDialog> {
        let world = self.world();
        world
            .windows
            .iter()
            .filter(|(_, open)| **open)
            .filter_map(|(window, _)| {
                let text = *world.window_texts.get(window)?;
                let (flags, speaker) = world.window_speakers.get(window).copied()?;
                let mut entry = self.entries.get(&text)?.clone();
                entry.tokens = self.fill_tags(&entry.tokens);
                Some(OpenDialog {
                    window: *window,
                    text,
                    entry,
                    layout: self.layouts.get(&text)?.clone(),
                    flags,
                    speaker,
                    choosing: self.choice_texts.contains(&text),
                })
            })
            .collect()
    }

    /// Replaces the tags that name game state with their text
    /// (`DialogBoxSymbols.ParseSingleConstantTextReplaceTag` and
    /// `ParseVariableTextReplaceTags`).
    fn fill_tags(
        &self,
        tokens: &[tantalus_assets::pc::mes::MesToken],
    ) -> Vec<tantalus_assets::pc::mes::MesToken> {
        use tantalus_assets::pc::mes::MesToken;
        let world = self.world();
        let text = |text: String| MesToken::Text { text };
        tokens
            .iter()
            .map(|token| {
                let MesToken::Tag(tag) = token else {
                    return token.clone();
                };
                let param = |index: usize| -> Option<i32> { tag.params.get(index)?.parse().ok() };
                match tag.code.as_str() {
                    "CharacterName" => {
                        let keyword = tag.params.first().map_or("", String::as_str);
                        text(crate::name_menu::display_name(
                            world,
                            keyword_character(keyword),
                        ))
                    }
                    // `[PTYn]`: the character in party slot n (1-based).
                    "Party" => {
                        let slot = param(0).unwrap_or(1).max(1) as usize - 1;
                        let id = world.party.get(slot).copied().unwrap_or(0);
                        text(crate::name_menu::display_name(world, id))
                    }
                    "Variable" => text(
                        world
                            .mes_values
                            .get(&param(0).unwrap_or(0))
                            .map_or(0, |v| *v)
                            .to_string(),
                    ),
                    "Item" => {
                        let id = world.mes_values.get(&param(0).unwrap_or(0)).copied();
                        text(
                            id.and_then(|id| self.data.item_names.get(&id))
                                .cloned()
                                .unwrap_or_default(),
                        )
                    }
                    _ => token.clone(),
                }
            })
            .collect()
    }

    fn enter(&mut self, map: i32, events: &mut Vec<PlayEvent>) -> Result<(), String> {
        let Some(name) = self.data.db.get(&map).cloned() else {
            return self.stop(format!("map {map} is not in the event DB"), events);
        };
        let binary = match load_event(&self.data.bundle, &format!("{name}.eb")) {
            Ok(binary) => binary,
            Err(error) => return self.stop(format!("{name}: {error}"), events),
        };
        let (pool, _) = decode_pool(&binary)?;
        let mut world = std::mem::take(&mut self.world);
        prepare_field_world(&mut world, map);
        if self.live_movies {
            let movies = self.data.movies.clone();
            let assets = self.data.streaming_assets.clone();
            world.fmv_frame_counts = Some(std::sync::Arc::new(move |disc, number| {
                let name = movies.get(&(disc, number))?;
                crate::movie::movie_file(&assets, name).map(|_| u32::MAX)
            }));
        }
        match self.data.backgrounds.get(&map) {
            Some(background) => match load_tile_animations(&self.data.field_bundles, background) {
                Ok((animations, cameras)) => world.provide_tile_animations(animations, cameras),
                Err(_) => world.clear_tile_animations(),
            },
            None => world.clear_tile_animations(),
        }
        if let Some(Ok(cameras)) = self
            .data
            .backgrounds
            .get(&map)
            .map(|background| load_cameras(&self.data.field_bundles, background))
        {
            world.provide_cameras(cameras);
        }
        let navigation = match self.data.backgrounds.get(&map) {
            Some(background) => match load_navigation(&self.data.field_bundles, background) {
                Ok(mesh) => {
                    let status = format!("{background}: {} triangles", mesh.triangles.len());
                    world.provide_navigation(mesh);
                    status
                }
                Err(error) => {
                    world.clear_navigation();
                    error
                }
            },
            None => {
                world.clear_navigation();
                "no background name".to_owned()
            }
        };
        match self.data.text.info(map) {
            Ok(info) => {
                world.dialog_signals = info.signals;
                self.choice_texts = info.choices;
                self.plain = info.plain;
                self.layouts = info.layouts;
                self.entries = info.entries;
            }
            Err(_) => {
                world.dialog_signals.clear();
                self.choice_texts.clear();
                self.plain.clear();
                self.layouts.clear();
                self.entries.clear();
            }
        }
        let mut session = FieldSession::start(map, world, pool)?;
        session
            .director
            .set_walk_bits(Some((pad::UP, pad::DOWN, pad::LEFT, pad::RIGHT)));
        session.director.set_walk_hold_bit(Some(pad::CANCEL));
        self.session = Some(session);
        self.audio
            .push((self.ticks, AudioCue::FieldStarted { map }));
        self.field_ready_due = Some(self.ticks);
        self.world_song_ride = None;
        self.shown.clear();
        self.cursor = 0;
        self.floor = None;
        events.push(PlayEvent::Field {
            map,
            name,
            navigation,
        });
        Ok(())
    }

    fn stop(&mut self, reason: String, events: &mut Vec<PlayEvent>) -> Result<(), String> {
        events.push(PlayEvent::Stopped(reason.clone()));
        self.stopped = Some(reason);
        Ok(())
    }

    fn party(&self) -> Vec<PartyFighter> {
        let world = self.world();
        // Every fight starts from the summons the game has seen.
        if let Some(battles) = &self.data.battles {
            battles.set_summons_seen(&world.summons_seen);
        }
        world
            .party
            .iter()
            .filter_map(|character| {
                let base = *self.data.battle_base.get(character)?;
                let stats = world.character_stats.get(character)?;
                Some(PartyFighter {
                    character: *character,
                    hp: stats.current_hp.max(0) as u32,
                    max_hp: stats.max_hp.max(0) as u32,
                    mp: stats.current_mp.max(0) as u32,
                    max_mp: stats.max_mp.max(0) as u32,
                    base,
                    level: self
                        .data
                        .menu
                        .level(world.character_exp.get(character).copied().unwrap_or(0))
                        as u8,
                    globals: PartyGlobals::from_globals(world.global_memory.bytes()),
                    row: u8::from(!world.back_row.contains(character)),
                    trance: world.trance.get(character).copied().unwrap_or(0),
                    equip: PartyFighter::loadout(world, *character).0,
                    support: PartyFighter::loadout(world, *character).1,
                    mastered: PartyFighter::mastered_abilities(world, *character),
                })
            })
            .collect()
    }

    /// Runs the main menu for this tick. True while it is open (the field
    /// is paused) or just opened.
    fn tick_menu(
        &mut self,
        held: i32,
        pressed: i32,
        events: &mut Vec<PlayEvent>,
    ) -> Result<bool, String> {
        if let Some(mut screen) = self.name_screen.take() {
            // `NameSettingUI`: the field stays paused until a name is confirmed.
            let world = match self.session.as_mut() {
                Some(session) => session.director.world_mut(),
                None => &mut self.world,
            };
            let result = screen.press(pressed, world);
            if let Some(id) = result.sound {
                self.audio.push((self.ticks, AudioCue::Effect { id }));
            }
            if !result.closed {
                self.name_screen = Some(screen);
            }
            return Ok(true);
        }
        if let Some(mut shop) = self.shop.take() {
            let mut open = true;
            if pressed != 0 {
                let world = match self.session.as_mut() {
                    Some(session) => session.director.world_mut(),
                    None => &mut self.world,
                };
                let before = shop.revision;
                open = shop.press(pressed, world, &self.data.menu, &self.data.shops);
                let sound = if std::mem::take(&mut shop.refused) {
                    Some(menu_sound::REFUSE)
                } else if pressed & (CONFIRM | pad::CIRCLE) != 0 {
                    Some(menu_sound::CONFIRM)
                } else if pressed & (pad::CANCEL | pad::CROSS) != 0 {
                    Some(menu_sound::CANCEL)
                } else if pressed & (pad::UP | pad::DOWN | pad::LEFT | pad::RIGHT) != 0
                    && shop.revision != before
                {
                    Some(menu_sound::CURSOR)
                } else {
                    None
                };
                if let Some(id) = sound {
                    self.audio.push((self.ticks, AudioCue::Effect { id }));
                }
            }
            if open {
                self.shop = Some(shop);
            }
            return Ok(true);
        }
        if let Some(mut menu) = self.menu.take() {
            let mut open = true;
            // `MainMenuUI.Update`: `UpdateTickTime` then `DisplayTime`, the clock runs while
            // the menu is up. Ticks are field ticks, 30 a second.
            menu.seconds = self.ticks / TICKS_PER_SECOND;
            if pressed != 0 {
                let tables = &self.data.menu;
                let world = match self.session.as_mut() {
                    Some(session) => session.director.world_mut(),
                    None => &mut self.world,
                };
                let before = menu.revision;
                open = menu.press(pressed, world, tables);
                // `FF9SFX_Play(103)` moves the cursor and confirms, `101` goes back, `102` refuses.
                let sound = if std::mem::take(&mut menu.refused) {
                    Some(menu_sound::REFUSE)
                } else if pressed & (CONFIRM | pad::CIRCLE) != 0 {
                    Some(menu_sound::CONFIRM)
                } else if pressed & (pad::CANCEL | pad::CROSS) != 0 {
                    Some(menu_sound::CANCEL)
                } else if pressed & (pad::UP | pad::DOWN | pad::LEFT | pad::RIGHT) != 0
                    && menu.revision != before
                {
                    Some(menu_sound::CURSOR)
                } else {
                    None
                };
                if let Some(id) = sound {
                    self.audio.push((self.ticks, AudioCue::Effect { id }));
                }
            }
            if let Some(action) = menu.action.take() {
                match action {
                    MenuAction::Save(slot) => {
                        let line = match self.save_slot(slot) {
                            Ok(()) => format!("Saved to slot {slot}."),
                            Err(error) => error,
                        };
                        menu.say(line);
                        menu.slots_stale = true;
                    }
                    MenuAction::Load(slot) => match self.load_slot(slot, events) {
                        Ok(()) => open = false,
                        Err(error) => menu.say(error),
                    },
                }
            }
            if menu.slots_stale {
                menu.slot_rows = self.slot_rows();
                menu.slots_stale = false;
                menu.revision += 1;
            }
            if open {
                self.menu = Some(menu);
            }
            return Ok(true);
        }
        if let Some(play) = self.world_play.as_ref() {
            // `WorldHUD.OnKeyMenu`: on the world map the main menu opens on foot only
            // (`WMUIData.ControlNo == 0`), not over the Chocograph list, and not while the
            // world script holds the menu off (`SetMenuControl`) or has a window up.
            let script_allows = play
                .script()
                .map(|script| script.world())
                .is_none_or(|world| {
                    !world.menu_disabled && !world.windows.values().any(|open| *open)
                });
            let opens = pressed & pad::START != 0
                && held & pad::START != 0
                && play.ride == 0
                && self.chocograph_screen.is_none()
                && script_allows;
            let moogle_save = self.session.as_mut().is_some_and(|session| {
                std::mem::take(&mut session.director.world_mut().save_requested)
            });
            if moogle_save {
                // The moogle's `MENU(4, 0)`: the save screen, as a field moogle opens it.
                let mut menu = FieldMenu::save_screen();
                menu.slot_rows = self.slot_rows();
                menu.slots_stale = false;
                self.menu = Some(menu);
                return Ok(true);
            }
            if opens && let Some(session) = self.session.as_mut() {
                // `MainMenuUI` has no Save command; the world map saves through its moogle.
                session.director.world_mut().save_point = false;
                self.menu = Some(self.main_menu());
            }
            return Ok(self.menu.is_some());
        }
        let Some(session) = self.session.as_mut() else {
            return Ok(false);
        };
        let world = session.director.world_mut();
        let allowed =
            world.user_control && !world.menu_disabled && !world.windows.values().any(|open| *open);
        if let Some(id) = world.name_requested
            && !world.windows.values().any(|open| *open)
        {
            // A script ran `MENU(1, character)` (`EventService.OpenNameMenu`).
            world.name_requested = None;
            self.name_screen = Some(crate::name_menu::NameMenu::new(world, id));
            return Ok(true);
        }
        if let Some(id) = world.shop_requested
            && !world.windows.values().any(|open| *open)
        {
            // A shop script ran `MENU(2, shop)`.
            world.shop_requested = None;
            self.shop = Some(ShopMenu::new(id, &self.data.menu, &self.data.shops));
            return Ok(true);
        }
        if world.save_requested && allowed {
            // A moogle asked for the save screen.
            world.save_requested = false;
            let mut menu = FieldMenu::save_screen();
            menu.slot_rows = self.slot_rows();
            menu.slots_stale = false;
            self.menu = Some(menu);
            return Ok(true);
        }
        if pressed & pad::START != 0 && allowed && held & pad::START != 0 {
            self.menu = Some(self.main_menu());
            return Ok(true);
        }
        Ok(false)
    }

    /// The main menu as it opens: the clock and the place it names
    /// (`MainMenuUI.DisplayGeneralInfo`: the field's location name, the world map area's name).
    fn main_menu(&self) -> FieldMenu {
        let mut menu = FieldMenu::new();
        menu.seconds = self.ticks / TICKS_PER_SECOND;
        menu.place = self.place_now(self.map().unwrap_or(0)).unwrap_or_default();
        menu
    }

    /// Folder the save slots live in.
    pub fn set_save_dir(&mut self, dir: PathBuf) {
        self.save_dir = dir;
    }

    /// Writes the game to `slot` (1-based): the map, the player's position
    /// and the world.
    pub fn save_slot(&mut self, slot: usize) -> Result<(), String> {
        if self.steam.is_some() {
            return self.save_steam_slot(slot);
        }
        let (map, position) = match self.disc_change {
            // A save between discs starts the next disc (the PSX's disc-change save).
            Some(change) => (change.field, None),
            None => (
                self.map().ok_or("there is no field to save")?,
                player_position(self.world()),
            ),
        };
        let place = self.place_now(map);
        let world = self.world();
        let save = PlaySave {
            format: SAVE_FORMAT,
            map,
            position,
            ticks: self.ticks,
            world: world.for_save(),
            place,
        };
        play_save::write(&self.save_dir, slot, &save)
    }

    /// `FF9.mapNameStr` now, which the save keeps as its preview's place
    /// (`SharedDataBytesStorage`: `Location = mapNameStr`): the world map area's name on the
    /// world map, else field `map`'s location name.
    fn place_now(&self, map: i32) -> Option<String> {
        if let Some(play) = &self.world_play {
            let area = tantalus_legacy::world::id_area(play.walker.status.id);
            return self.data.world_place_name(area).map(str::to_owned);
        }
        self.data.place_name(map).map(str::to_owned)
    }

    /// The place a save slot names: what it kept, else the field's location name, else (a
    /// world map save from before saves kept the area's name) "World Map", else the field's
    /// event id.
    fn slot_place(&self, kept: Option<&str>, map: i32) -> String {
        kept.or_else(|| self.data.place_name(map))
            .or_else(|| (9000..=9012).contains(&map).then_some("World Map"))
            .or_else(|| self.data.field_name(map))
            .unwrap_or("?")
            .to_owned()
    }

    /// Replaces the game with `slot`: enters the saved map with the saved
    /// world and puts the player back.
    pub fn load_slot(&mut self, slot: usize, events: &mut Vec<PlayEvent>) -> Result<(), String> {
        if self.steam.is_some() {
            return self.load_steam_slot(slot, events);
        }
        let save = play_save::read(&self.save_dir, slot)?.ok_or("that slot is empty")?;
        let mut world = save.world;
        world.copy_providers_from(&self.data.new_game);
        self.world = world;
        self.session = None;
        self.menu = None;
        self.shop = None;
        self.name_screen = None;
        self.stopped = None;
        self.disc_change = None;
        // A load leaves the world map for the saved field.
        self.world_play = None;
        self.chocograph_screen = None;
        self.title = None;
        self.ticks = save.ticks;
        self.pending_position = save.position;
        self.enter(save.map, events)?;
        // A save made on the world map (map 9000 to 9012) goes back to walking it, where its
        // world script puts the player (`FF9StateSystem.Load` -> `SceneDirector.Replace
        // ("WorldMap")`), instead of running the world script as a field.
        if (9000..=9012).contains(&save.map) && self.stopped.is_none() {
            self.world_script_map = save.map;
            // No field init follows: its `FieldReady` would stop the world song.
            self.field_ready_due = None;
            self.resume_world()?;
        }
        Ok(())
    }

    /// Starts from a save of a Steam save file (`--load-save`): the saved party, items, gil,
    /// event globals and the rest replace the new game, and the saved field is entered.
    /// Saves from the menu go to `<file>.tantalus.dat`, not to `path`.
    pub fn load_steam_save(
        &mut self,
        path: &Path,
        location: &tantalus_legacy::saves::LegacySaveLocation,
        events: &mut Vec<PlayEvent>,
    ) -> Result<Resume, String> {
        let save = steam_save::read_slot(path, location)?;
        self.steam = Some(SteamFile {
            source: path.to_path_buf(),
            output: path.with_extension("tantalus.dat"),
            slot: if location.is_autosave {
                0
            } else {
                location.slot_id
            },
            base: save.state.clone(),
        });
        self.enter_steam_save(save, events)
    }

    /// Where saves from the menu go, instead of the default copy next to the loaded file.
    pub fn set_steam_output(&mut self, path: PathBuf) {
        if let Some(steam) = self.steam.as_mut() {
            steam.output = path;
        }
    }

    fn enter_steam_save(
        &mut self,
        save: SteamSave,
        events: &mut Vec<PlayEvent>,
    ) -> Result<Resume, String> {
        let mut world = self.data.new_game.clone();
        let resume = steam_save::apply(&save.state, &mut world, &self.data.menu)?;
        self.world = world;
        self.session = None;
        self.menu = None;
        self.shop = None;
        self.name_screen = None;
        self.stopped = None;
        self.disc_change = None;
        // A load leaves the world map for the saved field.
        self.world_play = None;
        self.chocograph_screen = None;
        self.title = None;
        self.ticks = (save.state.play_time_seconds * TICKS_PER_SECOND as f64) as u64;
        self.pending_position = None;
        if let Some(steam) = self.steam.as_mut() {
            steam.base = save.state;
        }
        self.enter(resume.map, events)?;
        Ok(resume)
    }

    /// The file the save and load screens read: the output once it exists, else the source.
    fn steam_file(steam: &SteamFile) -> &Path {
        if steam.output.is_file() {
            &steam.output
        } else {
            &steam.source
        }
    }

    fn steam_location(
        steam: &SteamFile,
        slot: usize,
    ) -> tantalus_legacy::saves::LegacySaveLocation {
        tantalus_legacy::saves::LegacySaveLocation {
            is_autosave: false,
            slot_id: steam.slot,
            save_id: slot.saturating_sub(1) as i32,
        }
    }

    fn save_steam_slot(&mut self, slot: usize) -> Result<(), String> {
        let map = match self.disc_change {
            Some(change) => change.field,
            None => self.map().ok_or("there is no field to save")?,
        };
        let steam = self.steam.as_ref().ok_or("no save file")?;
        let state =
            steam_save::capture(self.world(), map, self.ticks, &self.data.menu, &steam.base)?;
        if !steam.output.exists() && steam.source.is_file() {
            // The first save copies the original file, so the other saves in it carry over.
            if let Some(dir) = steam.output.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
            }
            std::fs::copy(&steam.source, &steam.output)
                .map_err(|error| format!("cannot copy {}: {error}", steam.source.display()))?;
        }
        let place = self.data.field_name(map).unwrap_or("?").to_owned();
        steam_save::write_slot(
            &steam.output,
            &Self::steam_location(steam, slot),
            &state,
            &place,
        )?;
        if let Some(steam) = self.steam.as_mut() {
            steam.base = state;
        }
        Ok(())
    }

    fn load_steam_slot(&mut self, slot: usize, events: &mut Vec<PlayEvent>) -> Result<(), String> {
        let steam = self.steam.as_ref().ok_or("no save file")?;
        let save =
            steam_save::read_slot(Self::steam_file(steam), &Self::steam_location(steam, slot))?;
        self.enter_steam_save(save, events).map(|_| ())
    }

    fn steam_slot_rows(&self, steam: &SteamFile) -> Vec<SlotRow> {
        let bytes = std::fs::read(Self::steam_file(steam)).unwrap_or_default();
        (1..=SLOTS)
            .map(|slot| {
                let location = Self::steam_location(steam, slot);
                match steam_save::read_slot_bytes(&bytes, &location) {
                    Ok(save) => {
                        let state = &save.state;
                        // The party's character ids; the first is the leader
                        // (`SaveLoadUI.DisplayFileInfo`).
                        let party: Vec<i32> = state
                            .party
                            .members
                            .iter()
                            .flatten()
                            .map(|&index| i32::from(index))
                            .collect();
                        let lead = party
                            .first()
                            .and_then(|&index| state.party.characters.get(index as usize));
                        SlotRow::Filled(SlotFile {
                            leader: lead.map_or(String::new(), |record| record.name.clone()),
                            level: lead.map_or(1, |record| i32::from(record.level)),
                            place: self.slot_place(None, i32::from(state.field.fld_map_no)),
                            gil: u64::from(state.party.gil),
                            seconds: state.play_time_seconds as u64,
                            party,
                        })
                    }
                    Err(message) if message == steam_save::EMPTY_SLOT => SlotRow::Empty,
                    Err(_) => SlotRow::Unreadable,
                }
            })
            .collect()
    }

    /// Opens the load screen (a play window started without a field to
    /// resume).
    pub fn open_load_screen(&mut self) {
        let mut menu = FieldMenu::load_screen();
        menu.slot_rows = self.slot_rows();
        menu.slots_stale = false;
        self.menu = Some(menu);
    }

    /// One entry per save slot for the save and load screens. The play time is the save's
    /// `ticks` (or the Steam file's `play_time_seconds`), the same clock the main menu's Time
    /// window shows.
    pub fn slot_rows(&self) -> Vec<SlotRow> {
        if let Some(steam) = &self.steam {
            return self.steam_slot_rows(steam);
        }
        (1..=SLOTS)
            .map(|slot| match play_save::read(&self.save_dir, slot) {
                Ok(Some(save)) => {
                    let tables = &self.data.menu;
                    let lead = save.world.party.first().copied();
                    let level = lead.map_or(1usize, |id| {
                        tables.level(save.world.character_exp.get(&id).copied().unwrap_or(0))
                    });
                    SlotRow::Filled(SlotFile {
                        leader: lead.map_or(String::new(), |id| {
                            crate::name_menu::display_name(&save.world, id)
                        }),
                        level: level as i32,
                        place: self.slot_place(save.place.as_deref(), save.map),
                        gil: u64::from(save.world.gil),
                        seconds: save.ticks / TICKS_PER_SECOND,
                        party: save.world.party.iter().take(4).copied().collect(),
                    })
                }
                Ok(None) => SlotRow::Empty,
                Err(_) => SlotRow::Unreadable,
            })
            .collect()
    }

    /// Replaces the party (a test and demo aid; scripts set it in play).
    pub fn set_party(&mut self, ids: &[i32]) {
        self.world.party = ids.to_vec();
        if let Some(session) = self.session.as_mut() {
            session.director.world_mut().party = ids.to_vec();
        }
    }

    /// Gives `character` full AP in `ability` (a test aid for the ability
    /// screen); false when their preset lacks it.
    pub fn learn(&mut self, character: i32, ability: i32) -> bool {
        let Some(entry) = self
            .data
            .menu
            .abilities
            .learnable
            .get(&character)
            .and_then(|list| list.iter().find(|entry| entry.id == ability))
            .copied()
        else {
            return false;
        };
        let world = match self.session.as_mut() {
            Some(session) => session.director.world_mut(),
            None => &mut self.world,
        };
        world.apply_ap(character, ability, entry.max_ap, entry.max_ap);
        true
    }

    /// Lets the main menu save here, as after a moogle's `MENU(4, 0)` (a
    /// test aid).
    pub fn set_save_point(&mut self) {
        self.world.save_point = true;
        if let Some(session) = self.session.as_mut() {
            session.director.world_mut().save_point = true;
        }
    }

    /// Asks for shop `id` as a field script's `MENU(2, id)` does (a test
    /// aid).
    /// The party screen a field script opened, and its cursor, for the client to draw.
    pub fn party_screen(
        &self,
    ) -> Option<(
        &tantalus_legacy::party_menu::party::PartyInfo,
        crate::party_select::PartySelect,
    )> {
        let info = self.session.as_ref()?.director.world().party_menu_info()?;
        Some((info, self.party_select))
    }

    /// Walks the world with the pictures in `found` (bit `n` for picture
    /// `n`) found and a chocobo that reaches `reach`, and has the world
    /// script open the Chocograph list as its prompt does (a test aid).
    pub fn open_chocograph(
        &mut self,
        found: u32,
        reach: u8,
        events: &mut Vec<PlayEvent>,
    ) -> Result<(), String> {
        use tantalus_legacy::VariableType::Byte;
        {
            let world = self.world_mut_any();
            for index in 0..3u16 {
                let byte = (found >> (8 * index)) as u8;
                world.global_memory.set(187 + index, Byte, i32::from(byte));
            }
            world.global_memory.set(190, Byte, 1);
            world.global_memory.set(191, Byte, i32::from(reach));
        }
        self.start_world(None, events)?;
        if let Some(play) = self.world_play.as_mut() {
            play.script_mut_for_tests().world_mut().chocograph_requested = true;
        }
        Ok(())
    }

    /// Asks for the naming screen as `MENU(1, character)` does (a test aid).
    pub fn open_name(&mut self, id: i32) {
        self.world_mut_any().name_requested = Some(id);
    }

    pub fn open_shop(&mut self, id: i32) {
        self.world_mut_any().shop_requested = Some(id);
    }

    /// Asks for the save screen as a moogle's `MENU(4, 0)` does (a test aid).
    pub fn request_save(&mut self) {
        let world = self.world_mut_any();
        world.save_point = true;
        world.save_requested = true;
        world.user_control = true;
    }

    /// Sets the gil the party holds (a test aid).
    pub fn set_gil(&mut self, gil: u32) {
        self.world_mut_any().gil = gil;
    }

    /// Raises the script presentation state a capture wants to look at: the bubble icon, the
    /// "here" icon mode, a finished fade filter and the player's object scale (a test aid).
    pub fn show_presentation(
        &mut self,
        bubble: Option<i32>,
        here: i32,
        fade: Option<(i32, [i32; 3])>,
        player_scale: Option<i32>,
        ate_mode: Option<i32>,
    ) {
        let world = self.world_mut_any();
        world.ate_icon_mode = ate_mode.unwrap_or(0);
        if let (Some(scale), Some(player)) = (player_scale, world.player()) {
            world.actor_presentation.entry(player).or_default().scale = [scale; 3];
        }
        world.bubble_icon = bubble;
        world.here_icon = here;
        world.here_icon_timer = if here != 0 { 30 } else { 0 };
        world.fade_filter = fade.map(|(mode, color)| tantalus_game::field::world::FadeFilter {
            mode,
            frames: 16,
            color,
            elapsed: 16,
            from: [0; 3],
        });
    }

    fn world_mut_any(&mut self) -> &mut FieldWorld {
        match self.session.as_mut() {
            Some(session) => session.director.world_mut(),
            None => &mut self.world,
        }
    }

    /// Adds one of each item id (a test aid for the equip screen).
    pub fn give_items(&mut self, ids: &[i32]) {
        let world = match self.session.as_mut() {
            Some(session) => session.director.world_mut(),
            None => &mut self.world,
        };
        for id in ids {
            *world.items.entry(*id).or_insert(0) += 1;
        }
    }

    /// Drops every party member to a quarter of their HP (a test aid for
    /// the item screen).
    pub fn hurt_party(&mut self) {
        let world = match self.session.as_mut() {
            Some(session) => session.director.world_mut(),
            None => &mut self.world,
        };
        for stats in world.character_stats.values_mut() {
            stats.current_hp = (stats.max_hp / 4).max(1);
        }
    }

    /// The movie a script is playing, when [`Self::live_movies`] is on and its file exists.
    /// The field waits until [`Self::end_movie`].
    pub fn movie(&self) -> Option<crate::movie::MovieRequest> {
        if !self.live_movies {
            return None;
        }
        let fmv = &self.session.as_ref()?.director.world().fmv;
        if fmv.status != 6 {
            return None;
        }
        let name = self.data.movies.get(&(fmv.disc, fmv.number))?;
        Some(crate::movie::MovieRequest {
            disc: fmv.disc,
            number: fmv.number,
            name: name.clone(),
            path: crate::movie::movie_file(&self.data.streaming_assets, name)?,
        })
    }

    /// The field camera sound positions are projected through (`FF9FieldSoundGetPositionVolume`);
    /// without one they are centered.
    pub fn set_sound_camera(&mut self, camera: Option<tantalus_contracts::LegacyFieldCamera>) {
        self.sound_camera = camera;
    }

    /// `SetSoundPosition` and `SetSoundObjectPosition` as a cue with the camera projection done.
    fn sound_position_cue(
        camera: Option<&tantalus_contracts::LegacyFieldCamera>,
        world: &FieldWorld,
        event: &HostEvent,
    ) -> Option<AudioCue> {
        let camera = camera?;
        let (point, volume) = match event {
            HostEvent::SetSoundPosition { position, volume } => {
                (position.map(|value| value as f32), *volume)
            }
            HostEvent::SetSoundObjectPosition { object, volume } => {
                (world.actors.get(object)?.position, *volume)
            }
            _ => return None,
        };
        let (pos, vol) = crate::sound_view::position_volume(camera, point);
        Some(AudioCue::SoundPosition { pos, vol, volume })
    }

    /// The command menu's sound for one frame of input ([`crate::battle_sound::menu_cue`]).
    pub fn battle_menu_sound(
        &mut self,
        before: crate::battle_sound::MenuSnapshot,
        after: crate::battle_sound::MenuSnapshot,
        pressed: i32,
        sent: bool,
    ) {
        let confirm = pressed & pad::CONFIRM != 0;
        let cancel = pressed & pad::CANCEL != 0;
        if let Some(cue) = crate::battle_sound::menu_cue(before, after, confirm, cancel, sent) {
            self.audio.push((self.ticks, cue));
        }
    }

    /// Queues the sound of the movie that starts (`Some`) or stops (`None`) in the window.
    pub fn movie_sound(&mut self, resource: Option<String>) {
        self.audio.push((self.ticks, AudioCue::Movie { resource }));
    }

    /// The pad bits held when a view that reads the pad itself (the live battle, a movie)
    /// hands back to the game ticks: a button still held from it is not a new press on the
    /// next tick. Unity reads `GetKeyDown` each frame whatever scene is up, so the Confirm that
    /// closed the battle result is no press in the field it returns to.
    pub fn carry_held(&mut self, held: i32) {
        self.previous = held;
    }

    /// The movie finished or was skipped: the script's cinematic wait ends.
    pub fn end_movie(&mut self) {
        let map = self.map().unwrap_or(0);
        if let Some(session) = self.session.as_mut() {
            session.director.world_mut().fmv.finish(map);
        }
    }

    /// The Tetra Master game being played, for the window to draw.
    pub fn card_game(&self) -> Option<&tantalus_game::cards::CardGame> {
        self.card.as_ref()
    }

    /// One field tick of a card game (30 a second): the pad, two game ticks, the sounds, and
    /// the hand-back to the world when it ends.
    fn tick_card(&mut self, pressed: i32) {
        use tantalus_game::cards::Input;
        let Some(card) = self.card.as_mut() else {
            return;
        };
        for (bit, input) in [
            (pad::UP, Input::Up),
            (pad::DOWN, Input::Down),
            (pad::LEFT, Input::Left),
            (pad::RIGHT, Input::Right),
            (CONFIRM, Input::Confirm),
            (pad::CANCEL, Input::Cancel),
        ] {
            if pressed & bit != 0 {
                card.press(input);
            }
        }
        card.advance(2);
        if card.is_done()
            && let Some(session) = self.session.as_mut()
        {
            card.finish(session.director.world_mut());
        }
        for event in card.take_events() {
            self.audio.push((self.ticks, crate::card_play::cue(event)));
        }
        if self.card.as_ref().is_some_and(|card| card.is_done()) {
            self.card = None;
        }
    }

    /// Drains the sound requests made so far (script opcodes, field changes, battles, menu and
    /// choice presses), each with the game tick it happened on.
    pub fn take_audio(&mut self) -> Vec<(u64, AudioCue)> {
        let settings = &self.world().settings;
        let volumes = (settings.music_volume, settings.sound_volume);
        if self.sent_volumes != Some(volumes) {
            self.sent_volumes = Some(volumes);
            let (music, sound) = volumes;
            self.audio
                .insert(0, (self.ticks, AudioCue::Volumes { music, sound }));
        }
        if let Some(started) = self.field_ready_due
            && self.ticks > started
        {
            // The field's first tick ran its init (`StartEvents`).
            self.field_ready_due = None;
            self.audio.push((self.ticks, AudioCue::FieldReady));
        }
        std::mem::take(&mut self.audio)
    }

    /// The battle result screen closed (`BattleResultUI.Hide`): the end song stops and the song
    /// set aside comes back.
    pub fn battle_result_closed(&mut self) {
        if std::mem::take(&mut self.result_screen_open) {
            self.audio.push((self.ticks, AudioCue::BattleLeft));
        }
    }

    /// Asks for the world song when the ride changed (`w_movementChange`, `ff9.cs:5683`).
    fn request_world_song(&mut self) {
        let Some(play) = self.world_play.as_ref() else {
            self.world_song_ride = None;
            return;
        };
        let ride = play.ride;
        let music = tantalus_legacy::world::vehicle::VEHICLES
            .get(usize::from(ride))
            .map_or(0, |vehicle| vehicle.music);
        let previous = self.world_song_ride.and_then(|old| {
            tantalus_legacy::world::vehicle::VEHICLES
                .get(usize::from(old))
                .map(|vehicle| vehicle.music)
        });
        self.world_song_ride = Some(ride);
        if previous == Some(music) {
            return;
        }
        let scenario = self
            .world()
            .global_memory
            .get(0, tantalus_legacy::VariableType::UInt16)
            .unwrap_or(0);
        self.audio.push((
            self.ticks,
            AudioCue::WorldSong {
                song: world_song(music, scenario),
            },
        ));
    }

    /// True while the naming screen is open (the keyboard types into it).
    pub fn name_screen_open(&self) -> bool {
        self.name_screen.is_some()
    }

    /// Keyboard text for the open naming screen (`NameSettingUI`'s input
    /// field): `typed` is added (filtered), `backspaces` characters removed.
    /// `enter` is the field's Confirm (`OnKeyConfirm` -> `OnConfirmButtonClick`).
    pub fn name_input(&mut self, typed: &str, backspaces: usize, enter: bool) {
        let Some(screen) = self.name_screen.as_mut() else {
            return;
        };
        for _ in 0..backspaces {
            screen.backspace();
        }
        screen.type_text(typed);
        if enter {
            let world = match self.session.as_mut() {
                Some(session) => session.director.world_mut(),
                None => &mut self.world,
            };
            let result = screen.confirm(world);
            if let Some(id) = result.sound {
                self.audio.push((self.ticks, AudioCue::Effect { id }));
            }
            if result.closed {
                self.name_screen = None;
            }
        }
    }

    /// The world script's open windows (the moogle's words and choices) as one frame over the
    /// world map: the world has no field camera to hang the field's speech bubbles on, so each
    /// window is a plain box along the bottom with the choice hand. `None` off the world map or
    /// with no window up. The number changes with the text and the cursor row.
    pub fn world_dialog_frame(&self) -> Option<(u64, crate::field_menu::MenuFrame)> {
        use crate::field_menu::{MenuFrame, MenuText, MenuWindow, Tint};
        use tantalus_assets::pc::mes::MesToken;
        self.world_play.as_ref()?;
        let dialogs = self.open_dialogs();
        if dialogs.is_empty() {
            return None;
        }
        const LINE: f32 = 12.0;
        const INDENT: f32 = 18.0;
        let mut frame = MenuFrame::default();
        let mut revision = 0u64;
        let mut bottom = 214.0;
        for dialog in dialogs.iter().rev() {
            // Rows of the text: `[CHOO]` starts the choice rows, `[MOVE=n,0]` indents one.
            let mut rows: Vec<(String, bool)> = vec![(String::new(), false)];
            let mut first_choice = None;
            for token in &dialog.entry.tokens {
                match token {
                    MesToken::Text { text } => {
                        for (index, part) in text.split('\n').enumerate() {
                            if index > 0 {
                                rows.push((String::new(), false));
                            }
                            if let Some(row) = rows.last_mut() {
                                row.0.push_str(part);
                            }
                        }
                    }
                    MesToken::Tag(tag) if tag.code == "Choice" => {
                        first_choice = Some(rows.len() - 1);
                    }
                    MesToken::Tag(tag) if tag.code == "MoveCaret" => {
                        if let Some(row) = rows.last_mut() {
                            row.1 = tag.params.first().is_some_and(|x| x != "0");
                        }
                    }
                    _ => {}
                }
            }
            while rows.last().is_some_and(|(text, _)| text.is_empty()) && rows.len() > 1 {
                rows.pop();
            }
            let widest = rows
                .iter()
                .map(|(text, indented)| {
                    text.chars().count() as f32 * 6.5 + if *indented { INDENT } else { 0.0 }
                })
                .fold(60.0_f32, f32::max);
            let w = (widest + 22.0).min(304.0);
            let h = rows.len() as f32 * LINE + 12.0;
            let (x, y) = ((320.0 - w) / 2.0, bottom - h);
            bottom = y - 4.0;
            let texts = rows
                .iter()
                .enumerate()
                .map(|(row, (text, indented))| MenuText {
                    x: 11.0 + if *indented { INDENT } else { 0.0 },
                    y: 6.0 + row as f32 * LINE,
                    text: text.clone(),
                    tint: Tint::White,
                    right: false,
                })
                .collect();
            frame.windows.push(MenuWindow { x, y, w, h, texts });
            revision = revision
                .wrapping_mul(31)
                .wrapping_add(u64::from(dialog.text));
            if let Some(first) = first_choice
                && dialog.choosing
            {
                let choices = rows.len() - first;
                let pick = usize::try_from(self.cursor.max(0))
                    .unwrap_or(0)
                    .min(choices.saturating_sub(1));
                frame.pointer =
                    Some((x + 8.0, y + 6.0 + (first + pick) as f32 * LINE + LINE / 2.0));
                revision = revision.wrapping_mul(31).wrapping_add(pick as u64 + 1);
            }
        }
        Some((revision | 1 << 43, frame))
    }

    /// The main menu to draw, with a number that changes when it does.
    pub fn menu_frame(&self) -> Option<(u64, crate::field_menu::MenuFrame)> {
        if let Some(screen) = &self.chocograph_screen {
            // Offset so it never matches a shop's or the menu's revision.
            return Some((screen.revision | 1 << 40, screen.frame()));
        }
        if let Some(screen) = &self.name_screen {
            // Offset so it never matches another screen's revision.
            return Some((screen.revision | 1 << 41, screen.frame()));
        }
        if let Some(shop) = &self.shop {
            return Some((
                shop.revision,
                shop.frame(self.world(), &self.data.menu, &self.data.shops),
            ));
        }
        if let Some(menu) = self.menu.as_ref() {
            // The main screen's clock redraws it each second.
            let clock = if menu.shows_clock() {
                (menu.seconds & 0x3F_FFFF) << 42
            } else {
                0
            };
            return Some((
                menu.revision | clock,
                menu.frame(self.world(), &self.data.menu),
            ));
        }
        let (title, _) = self.title.as_ref()?;
        Some((title.revision | 1 << 41, title.frame()))
    }

    /// One field tick (30 a second) with `held` pad bits.
    pub fn tick(&mut self, held: i32) -> Result<Vec<PlayEvent>, String> {
        let mut events = Vec::new();
        if self.stopped.is_some() || self.pending.is_some() || self.movie().is_some() {
            return Ok(events);
        }
        let pressed = held & !self.previous;
        self.previous = held;
        self.ticks += 1;
        if self.card.is_some() {
            self.tick_card(pressed);
            return Ok(events);
        }
        if self.disc_change.is_some() {
            self.tick_disc_change(held, pressed, &mut events)?;
            return Ok(events);
        }
        if self.game_over.is_some() {
            self.tick_game_over(pressed);
            return Ok(events);
        }
        if self.title.is_some() {
            self.tick_title(held, pressed, &mut events)?;
            return Ok(events);
        }
        if self.tick_menu(held, pressed, &mut events)? {
            return Ok(events);
        }
        if self.world_play.is_some() {
            self.tick_world(held, pressed, &mut events)?;
            return Ok(events);
        }
        let map = self.map().unwrap_or(0);
        let Some(session) = self.session.as_mut() else {
            return Ok(events);
        };
        // The party screen (`PARTYMENU`) takes the pad while it is open; the waiting script goes
        // on once it closes.
        let party_screen = session.director.world().party_menu_open();
        if party_screen {
            let world = session.director.world_mut();
            if world.party_menu_info().is_none() {
                // Opened before this build kept the screen's state: leave with the party as is.
                world.close_party_menu();
            } else if let Some(event) = self.party_select.press(world, pressed) {
                use crate::party_select::PartySelectEvent;
                let id = match event {
                    PartySelectEvent::Refused => menu_sound::REFUSE,
                    PartySelectEvent::Moved => menu_sound::CURSOR,
                    _ => menu_sound::CONFIRM,
                };
                self.audio.push((self.ticks, AudioCue::Effect { id }));
            }
        }
        // A confirm press lasts one tick; holding it does not repeat. The party screen keeps
        // the pad from the field.
        let mut input = if party_screen { 0 } else { held & !CONFIRM };
        let world = session.director.world();
        let open: Vec<(u8, u16)> = world
            .windows
            .iter()
            .filter(|(_, open)| **open)
            .filter_map(|(window, _)| Some((*window, *world.window_texts.get(window)?)))
            .collect();
        let choosing = open
            .iter()
            .any(|(_, text)| self.choice_texts.contains(text));
        if choosing {
            let before = self.cursor;
            if pressed & pad::UP != 0 {
                self.cursor = (self.cursor - 1).max(0);
            }
            if pressed & pad::DOWN != 0 {
                self.cursor += 1;
            }
            if self.cursor != before {
                self.audio.push((
                    self.ticks,
                    AudioCue::Effect {
                        id: menu_sound::CURSOR,
                    },
                ));
            }
        }
        if !party_screen && pressed & pad::SQUARE != 0 {
            // `ETb.KeyOn` sees the logical Special control (square) as
            // 0x80000 for the frame it is pressed (`EventInput.ProcessInput`,
            // `GetKeyMaskFromControl`): Chocobo Hot and Cold's dig.
            input |= SPECIAL_KEY;
        }
        if !party_screen && pressed & CONFIRM != 0 {
            input |= CONFIRM;
            for (window, text) in &open {
                if PROMPT_KEYS
                    .iter()
                    .any(|(on, id, _)| *on == map && id == text)
                {
                    continue;
                }
                if self.choice_texts.contains(text) {
                    session.director.world_mut().dialog_choice = self.cursor;
                    self.audio.push((
                        self.ticks,
                        AudioCue::Effect {
                            id: menu_sound::CONFIRM,
                        },
                    ));
                }
                session.director.world_mut().close_window(*window);
            }
            self.cursor = 0;
        }
        let before = player_position(session.director.world());
        let step = session.tick(input)?;
        if let Some(opponent) = session.director.world_mut().pending_card_game.take()
            && let Some(tables) = self.data.cards.clone()
        {
            self.card = tantalus_game::cards::CardGame::start(
                session.director.world_mut(),
                &tables,
                opponent,
            );
        }
        for event in session.director.world_mut().take_sound_events() {
            let cue = Self::sound_position_cue(
                self.sound_camera.as_ref(),
                session.director.world(),
                &event,
            );
            self.audio
                .push((self.ticks, cue.unwrap_or(AudioCue::Script(event))));
        }
        walk_on_walkmesh(session.director.world_mut(), before, &mut self.floor);
        if let Some(position) = self.pending_position {
            let world = session.director.world_mut();
            if let Some(actor) = world
                .player()
                .and_then(|player| world.actors.get_mut(&player))
            {
                actor.position = position;
                self.pending_position = None;
                self.floor = None;
            }
        }
        events.extend(
            step.started
                .iter()
                .map(|&(entry, tag)| PlayEvent::Script { entry, tag }),
        );

        let world = session.director.world();
        let now: BTreeMap<u8, u16> = world
            .windows
            .iter()
            .filter(|(_, open)| **open)
            .filter_map(|(window, _)| Some((*window, *world.window_texts.get(window)?)))
            .collect();
        for (window, text) in &now {
            if self.shown.get(window) != Some(text) {
                events.push(PlayEvent::Dialog {
                    window: *window,
                    text: *text,
                    line: self.plain.get(text).cloned().unwrap_or_default(),
                    choice: self.choice_texts.contains(text),
                });
            }
        }
        self.shown = now;

        if !step.encounters.is_empty() && self.live_battles && self.data.battles.is_some() {
            self.pending = Some(PendingBattles {
                requests: step.encounters.clone(),
                fight: None,
                next_field: step.next_field,
                world_map: step.world_map,
                stage: None,
            });
            self.start_pending(&mut events);
            return Ok(events);
        }
        if !step.encounters.is_empty() {
            let party = self.party();
            let mut stages = Vec::new();
            let session = self.session.as_mut().expect("session");
            for request in &step.encounters {
                self.audio.push((
                    self.ticks,
                    AudioCue::BattleStarted {
                        map,
                        battle: request.battle,
                        world: false,
                    },
                ));
                let (outcome, no_win_pose) = match &self.data.battles {
                    Some(battles) => {
                        let items = session.director.world().items.clone();
                        let mut report = battles.fight_with_items(request, &party, &items);
                        save_party(&report, session.director.world_mut());
                        pay_rewards(&report, session.director.world_mut());
                        stages.push(report.stage.take());
                        let no_win_pose = report.no_win_pose;
                        (
                            report.outcome.unwrap_or_else(|| "no result".to_owned()),
                            no_win_pose,
                        )
                    }
                    None => ("skipped".to_owned(), false),
                };
                self.audio.push((
                    self.ticks,
                    AudioCue::BattleEnded {
                        end: crate::play_audio::BattleEnd::from_outcome(&outcome),
                        no_win_pose,
                    },
                ));
                self.audio.push((self.ticks, AudioCue::BattleLeft));
                events.push(PlayEvent::Battle {
                    battle: request.battle,
                    outcome,
                });
            }
            session.resume_after_battle()?;
            if let Some(stage) = stages.into_iter().flatten().last() {
                self.last_battle = Some(stage);
            }
        }

        // `GAMEOVER` (`FLOW_STATE_GAMEOVER`, `HonoluluFieldMain.cs:259`): the field stops
        // (`shutdownField`) and the game over screen opens (`EventService.OpenGameOver`).
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.director.world().game_over)
        {
            self.menu = None;
            self.game_over = Some(GameOverScreen {
                stage: GameOverStage::FadeIn,
                ticks: 0,
            });
            events.push(PlayEvent::GameOver);
            return Ok(events);
        }
        // `DISCCHANGE` waits (`FLOW_STATE_WAIT`) while the scene changes to the next disc.
        if let Some((field, disc)) = self
            .session
            .as_mut()
            .and_then(|session| session.director.world_mut().disc_change.take())
        {
            let field = i32::from(field);
            events.push(PlayEvent::DiscChange { field, disc });
            self.disc_change = Some(DiscChange {
                field,
                disc,
                stage: DiscStage::Picture,
                ticks: 0,
            });
            return Ok(events);
        }
        self.leave(step.next_field, step.world_map, &mut events)?;
        Ok(events)
    }

    /// Puts the title screen up (`TitleUI`): what was on screen waits behind it, and New Game
    /// starts a new game on `start`. The game over screen comes here too
    /// (`GameOverUI.GoToTitleScene`).
    pub fn open_title(&mut self, start: i32) {
        let saved = self.latest_slot().is_some();
        let screen = if self.ps1_title {
            crate::title_menu::TitleScreen::ps1(saved)
        } else {
            crate::title_menu::TitleScreen::new(saved)
        };
        self.title = Some((screen, start));
        self.menu = None;
        // `TitleUI.Show` with the splash screen (at launch, and after a game over since every
        // game start sets `SplashScreenEnabled`): `SoundLib.StopAllSounds(true)`.
        self.audio.push((self.ticks, AudioCue::TitleShown));
    }

    /// Makes the title the PS1's (the PS1 skin with the disc's title art).
    pub fn set_ps1_title(&mut self, on: bool) {
        self.ps1_title = on;
    }

    /// Whether the title up is the PS1's.
    pub fn title_is_ps1(&self) -> bool {
        self.title.as_ref().is_some_and(|(screen, _)| screen.ps1)
    }

    /// Whether the title screen is up.
    pub fn title_open(&self) -> bool {
        self.title.is_some()
    }

    /// The save slot written last (Continue loads it; the play window has no autosave).
    pub fn latest_slot(&self) -> Option<usize> {
        if self.steam.is_some() {
            return None;
        }
        (1..=SLOTS)
            .filter_map(|slot| {
                let path = play_save::slot_path(&self.save_dir, slot);
                let written = std::fs::metadata(&path).ok()?.modified().ok()?;
                Some((written, slot))
            })
            .max()
            .map(|(_, slot)| slot)
    }

    /// One tick of the title: its menu, or the load screen it opened (cancel goes back).
    fn tick_title(
        &mut self,
        held: i32,
        pressed: i32,
        events: &mut Vec<PlayEvent>,
    ) -> Result<(), String> {
        use crate::title_menu::TitleChoice;
        if self.menu.is_some() {
            // Load Game's screen; a load closes the title (`load_slot`).
            self.tick_menu(held, pressed, events)?;
            return Ok(());
        }
        let Some((mut screen, start)) = self.title.take() else {
            return Ok(());
        };
        let choice = if pressed != 0 {
            let before = screen.cursor;
            let choice = screen.press(pressed);
            if screen.cursor != before {
                self.audio.push((
                    self.ticks,
                    AudioCue::Effect {
                        id: menu_sound::CURSOR,
                    },
                ));
            }
            choice
        } else {
            None
        };
        match choice {
            // `OnContinueButtonClick`: sound 1261 and the save, or the refusal.
            Some(TitleChoice::Continue) => match self.latest_slot() {
                Some(slot) => {
                    self.audio.push((self.ticks, AudioCue::Effect { id: 1261 }));
                    return self.load_slot(slot, events);
                }
                None => self.audio.push((
                    self.ticks,
                    AudioCue::Effect {
                        id: menu_sound::REFUSE,
                    },
                )),
            },
            Some(TitleChoice::NewGame) => {
                // `OnNewGameButtonClick`: sound 3096, `ReInitStateSystem`, the opening field.
                self.audio.push((self.ticks, AudioCue::Effect { id: 3096 }));
                self.world = self.data.new_game.clone();
                self.session = None;
                self.world_play = None;
                self.menu = None;
                self.shop = None;
                self.name_screen = None;
                self.stopped = None;
                self.chocograph_screen = None;
                self.ticks = 0;
                self.pending_position = None;
                return self.enter(start, events);
            }
            Some(TitleChoice::LoadGame) => {
                // `OnLoadGameButtonClick`: sound 103 and the load screen.
                self.audio.push((
                    self.ticks,
                    AudioCue::Effect {
                        id: menu_sound::CONFIRM,
                    },
                ));
                self.open_load_screen();
            }
            None => {}
        }
        self.title = Some((screen, start));
        Ok(())
    }

    /// The game over screen's picture opacity while it is up (a field script's `GAMEOVER`).
    pub fn game_over_alpha(&self) -> Option<f32> {
        self.game_over.as_ref().map(GameOverScreen::alpha)
    }

    /// One tick of the game over screen: the picture fades in, waits for Confirm
    /// (`GameOverUI.OnKeyConfirm`), fades out, and the title comes up
    /// (`GameOverUI.Hide` -> `SceneDirector.Replace("Title")`).
    fn tick_game_over(&mut self, pressed: i32) {
        let Some(mut screen) = self.game_over else {
            return;
        };
        screen.ticks += 1;
        match screen.stage {
            GameOverStage::FadeIn if screen.ticks >= GAME_OVER_FADE_TICKS => {
                screen.stage = GameOverStage::Wait;
                screen.ticks = 0;
            }
            GameOverStage::Wait if pressed & CONFIRM != 0 => {
                screen.stage = GameOverStage::FadeOut;
                screen.ticks = 0;
            }
            GameOverStage::FadeOut if screen.ticks >= GAME_OVER_FADE_TICKS => {
                self.game_over = None;
                self.open_title(70);
                return;
            }
            _ => {}
        }
        self.game_over = Some(screen);
    }

    /// The disc change in progress, if any.
    pub fn disc_change(&self) -> Option<&DiscChange> {
        self.disc_change.as_ref()
    }

    /// The disc picture to draw now and its opacity (`changetodisc<N>.png`, Memoria's
    /// `EmbeddedAsset/UI/Sprites`).
    pub fn disc_picture(&self) -> Option<(PathBuf, f32)> {
        let change = self.disc_change.as_ref()?;
        let disc = change.picture()?;
        Some((
            self.data.sprites.join(format!("changetodisc{disc}.png")),
            change.alpha(),
        ))
    }

    /// One tick of a disc change: the picture until a press (`SceneDirector`'s wait for any
    /// control), its fade-out, then the save screen; the next disc's first field once that
    /// closes.
    fn tick_disc_change(
        &mut self,
        held: i32,
        pressed: i32,
        events: &mut Vec<PlayEvent>,
    ) -> Result<(), String> {
        let Some(mut change) = self.disc_change else {
            return Ok(());
        };
        change.ticks += 1;
        let mut open_save = false;
        match change.stage {
            DiscStage::Picture => {
                if change.picture().is_none() {
                    open_save = true;
                } else if change.ticks >= DISC_FADE_TICKS && pressed != 0 {
                    change.stage = DiscStage::FadeOut;
                    change.ticks = 0;
                }
            }
            DiscStage::FadeOut => open_save = change.ticks >= DISC_FADE_TICKS,
            DiscStage::Save => {
                if self.menu.is_some() {
                    self.tick_menu(held, pressed, events)?;
                }
                if self.menu.is_none() {
                    // Saved or not, the next disc starts.
                    self.disc_change = None;
                    if let Some(session) = self.session.take() {
                        self.world = session.director.world().clone();
                    }
                    self.world.save_requested = false;
                    return self.enter(change.field, events);
                }
            }
        }
        if open_save {
            change.stage = DiscStage::Save;
            change.ticks = 0;
            let mut menu = FieldMenu::save_screen();
            menu.slot_rows = self.slot_rows();
            menu.slots_stale = false;
            self.menu = Some(menu);
        }
        self.disc_change = Some(change);
        Ok(())
    }

    /// What a field's `WORLD` call does (`ChangeWorldMap` to `world_map`): the next scripted
    /// `world_exits` location, else (with `world_mode`) the world map is walked.
    pub fn leave_for_world(
        &mut self,
        world_map: i32,
        events: &mut Vec<PlayEvent>,
    ) -> Result<(), String> {
        self.leave(None, Some(world_map), events)
    }

    /// The field change a tick's scripts asked for, if any.
    fn leave(
        &mut self,
        next_field: Option<i32>,
        world_map: Option<i32>,
        events: &mut Vec<PlayEvent>,
    ) -> Result<(), String> {
        if next_field.is_none()
            && self.world_mode
            && self.world_exits.is_empty()
            && world_map.is_some()
        {
            if let Some(map) = world_map.filter(|map| (9000..=9012).contains(map))
                && self.data.db.contains_key(&map)
            {
                self.world_script_map = map;
            }
            return self.start_world(None, events);
        }
        let session = self.session.as_mut().expect("session");
        let mut next = next_field;
        if next.is_none()
            && let Some(world_map) = world_map
        {
            let mut world = session.director.world().clone();
            let location = (!self.world_exits.is_empty()).then(|| self.world_exits.remove(0));
            let outcome = match (location, self.data.db.get(&world_map)) {
                (Some(location), Some(name)) => {
                    world_exit(&self.data.bundle, name, location, &mut world)?
                }
                _ => None,
            };
            events.push(PlayEvent::WorldMap {
                world_map,
                location,
                next: outcome,
            });
            match outcome {
                Some(field) => {
                    session.director.world_mut().clone_from(&world);
                    next = Some(field);
                }
                None => {
                    return self.stop(format!("world map {world_map}: no field to go to"), events);
                }
            }
        }
        if let Some(next) = next {
            let session = self.session.take().expect("session");
            self.world = session.director.world().clone();
            self.enter(next, events)?;
        }
        Ok(())
    }

    /// The battle being fought live, if any ([`Self::live_battles`]).
    /// The Config screen's values of the running game.
    pub fn settings(&self) -> Option<&tantalus_game::field::settings::GameSettings> {
        Some(&self.session.as_ref()?.director.world().settings)
    }

    pub fn battle(&self) -> Option<&LiveFight> {
        self.pending.as_ref()?.fight.as_ref()
    }

    /// Test aid for the frame-rate bench: starts battle `scene` as a live
    /// fight over the current field.
    pub fn bench_battle(&mut self, scene: i32) -> Vec<PlayEvent> {
        let mut events = Vec::new();
        self.pending = Some(PendingBattles {
            requests: vec![tantalus_game::field::EncounterRequest {
                rush: 0,
                group: None,
                battle: scene as _,
                steiner_state: false,
                timer: None,
                pattern_roll: 0,
            }],
            fight: None,
            next_field: None,
            world_map: None,
            stage: None,
        });
        self.start_pending(&mut events);
        events
    }

    /// Test aid for captures: fights battle `scene` to its end on the
    /// stand-in policy at once and keeps its stage in
    /// [`PlayGame::last_battle`], as an encounter does.
    pub fn bench_fight(&mut self, scene: i32) -> Vec<PlayEvent> {
        let party = self.party();
        let (Some(session), Some(battles)) = (self.session.as_mut(), self.data.battles.as_ref())
        else {
            return Vec::new();
        };
        let request = tantalus_game::field::EncounterRequest {
            rush: 0,
            group: None,
            battle: scene as _,
            steiner_state: false,
            timer: None,
            pattern_roll: 0,
        };
        let items = session.director.world().items.clone();
        let mut report = battles.fight_with_items(&request, &party, &items);
        self.last_battle = report.stage.take();
        vec![PlayEvent::Battle {
            battle: request.battle,
            outcome: report.outcome.unwrap_or_else(|| "no result".to_owned()),
        }]
    }

    /// Test aid for the frame-rate bench: starts a Tetra Master game
    /// against `opponent`.
    pub fn bench_card_game(&mut self, opponent: u16) {
        if let (Some(session), Some(tables)) = (self.session.as_mut(), self.data.cards.clone()) {
            self.card = tantalus_game::cards::CardGame::start(
                session.director.world_mut(),
                &tables,
                opponent,
            );
        }
    }

    /// Name of item `id` from the game text.
    pub fn item_name(&self, id: i32) -> Option<&str> {
        self.data.item_names.get(&id).map(String::as_str)
    }

    /// Name of ability `id` (an active ability, or 192 + a support ability).
    pub fn ability_name(&self, id: i32) -> String {
        self.data.menu.abilities.name(id)
    }

    /// Name of Tetra Master card `id` (`FF9TextTool.CardName`).
    pub fn card_name(&self, id: u32) -> Option<&str> {
        let id = i32::try_from(id).ok()?;
        self.data.menu.card_names.get(&id).map(String::as_str)
    }

    /// Level of a character with `exp` total EXP (`ff9level.CharacterLevelUps`).
    pub fn level_of(&self, exp: u64) -> usize {
        self.data.menu.level(exp)
    }

    /// A sound of the battle result screen (`BattleResultUI`: level up 683, ability learned
    /// 1043, the count's end 103).
    pub fn result_sound(&mut self, id: i32) {
        self.audio.push((self.ticks, AudioCue::Effect { id }));
    }

    /// Starts the first pending battle that sets up; one that cannot start
    /// is reported and skipped. With none left, the field resumes.
    fn start_pending(&mut self, events: &mut Vec<PlayEvent>) {
        let party = self.party();
        let on_world = self.world_play.is_some();
        let map = if on_world {
            self.world_script_map
        } else {
            self.map().unwrap_or(0)
        };
        loop {
            let Some(pending) = self.pending.as_mut() else {
                return;
            };
            let Some(request) = pending.requests.first().cloned() else {
                break;
            };
            let session = self.session.as_ref().expect("session");
            let items = session.director.world().items.clone();
            let battles = self.data.battles.as_ref().expect("battle data");
            match battles.start(&request, &party, &items) {
                Ok(mut fight) => {
                    if let Ok(text) = self.data.text.battle_text(request.battle) {
                        fight.name_enemies(&text);
                    }
                    fight.apply_settings(&session.director.world().settings);
                    pending.fight = Some(fight);
                    self.audio.push((
                        self.ticks,
                        AudioCue::BattleStarted {
                            map,
                            battle: request.battle,
                            world: on_world,
                        },
                    ));
                    self.result_screen_open = true;
                    return;
                }
                Err(report) => {
                    pending.requests.remove(0);
                    events.push(PlayEvent::Battle {
                        battle: request.battle,
                        outcome: report
                            .outcome
                            .clone()
                            .unwrap_or_else(|| "no result".to_owned()),
                    });
                }
            }
        }
        // Every battle is over: the field goes on as `tick` would have.
        let pending = self.pending.take().expect("pending");
        // A live fight's result screen sends it when it closes.
        if !self.result_screen_open {
            self.audio.push((self.ticks, AudioCue::BattleLeft));
        }
        if let Some(stage) = pending.stage {
            self.last_battle = Some(stage);
        }
        // Walking the world map, the field script is not waiting on the fight.
        let resumed = if self.world_play.is_some() {
            Ok(())
        } else {
            self.session
                .as_mut()
                .expect("session")
                .resume_after_battle()
        }
        .and_then(|()| self.leave(pending.next_field, pending.world_map, events));
        if let Err(error) = resumed {
            let _ = self.stop(error, events);
        }
    }

    /// The live battle, to send it the flee key.
    pub fn battle_mut(&mut self) -> Option<&mut LiveFight> {
        self.pending.as_mut()?.fight.as_mut()
    }

    /// One battle tick (15 a second) of the live battle, with `command` for
    /// a ready party member. When the battle ends, its result is carried
    /// back to the field as [`Self::tick`] does, and the next battle the
    /// field started begins, or the field resumes.
    pub fn battle_tick(&mut self, command: Option<BattleCommand>) -> Vec<PlayEvent> {
        let mut events = Vec::new();
        let Some(fight) = self
            .pending
            .as_mut()
            .and_then(|pending| pending.fight.as_mut())
        else {
            return events;
        };
        let more = fight.step(FightControl::Player(command));
        let effects = self.data.effect_sounds.get_or_init(|| {
            crate::battle_sound::EffectSounds::load(&self.data.game_data, &self.data.actions_csv)
                .map(|sounds| {
                    sounds
                        .with_plugin(self.data.plugin.clone())
                        .with_cast_leads(std::sync::Arc::new(crate::battle_effects::cast_leads()))
                })
                .map_err(|error| eprintln!("battle sounds: no spell sounds: {error}"))
                .ok()
        });
        let cues = self
            .battle_sounds
            .step(fight.stage(), fight.party_len(), effects.as_ref());
        self.audio
            .extend(cues.into_iter().map(|cue| (self.ticks, cue)));
        if more {
            return events;
        }
        let pending = self.pending.as_mut().expect("pending");
        let fight = pending.fight.take().expect("fight");
        let request = pending.requests.remove(0);
        let battles = self.data.battles.as_ref().expect("battle data");
        let mut report = fight.finish(battles);
        let session = self.session.as_mut().expect("session");
        let world = session.director.world_mut();
        save_party(&report, world);
        let tables = &self.data.menu;
        let levels = |world: &FieldWorld| -> Vec<usize> {
            report
                .reward_exp
                .iter()
                .map(|(character, _)| {
                    tables.level(world.character_exp.get(character).copied().unwrap_or(0))
                })
                .collect()
        };
        let before = levels(world);
        let learned_before = world.learned_abilities.clone();
        pay_rewards(&report, world);
        let after = levels(world);
        let victory = report.outcome.as_deref() == Some("victory");
        // The abilities each member learned from the AP (`BattleResultUI.ApLearned`), in the
        // order `AddAp` meets them.
        let learned: Vec<(i32, Vec<i32>)> = report
            .reward_ap_lines
            .iter()
            .filter(|_| victory)
            .map(|(character, _, goals)| {
                let had = learned_before.get(character);
                let now = world.learned_abilities.get(character);
                let new = goals
                    .iter()
                    .map(|&(ability, _)| ability)
                    .filter(|ability| {
                        now.is_some_and(|set| set.contains(ability))
                            && !had.is_some_and(|set| set.contains(ability))
                    })
                    .fold(Vec::new(), |mut list, ability| {
                        if !list.contains(&ability) {
                            list.push(ability);
                        }
                        list
                    });
                (*character, new)
            })
            .filter(|(_, new)| !new.is_empty())
            .collect();
        let party = world.party.clone();
        self.last_results = Some(BattleResults {
            party,
            learned,
            outcome: report.outcome.clone().unwrap_or_default(),
            gil: if victory { report.reward_gil } else { 0 },
            ap: if victory { report.reward_ap } else { 0 },
            exp: if victory {
                report
                    .reward_exp
                    .iter()
                    .zip(before.iter().zip(&after))
                    .map(|(&(character, exp), (&from, &to))| (character, exp, from, to))
                    .collect()
            } else {
                Vec::new()
            },
            items: if victory {
                report.reward_items.clone()
            } else {
                Vec::new()
            },
            stolen: report.items_stolen.clone(),
            card: report.reward_card.filter(|_| victory),
        });
        // The result screen's sounds (level up 683, ability learned 1043, the end of the count
        // 103) play from the play window as its count-up reaches them (`result_sound`).
        if let Some(stage) = report.stage.take() {
            pending.stage = Some(stage);
        }
        let no_win_pose = report.no_win_pose;
        let outcome = report.outcome.unwrap_or_else(|| "no result".to_owned());
        self.audio.push((
            self.ticks,
            AudioCue::BattleEnded {
                end: crate::play_audio::BattleEnd::from_outcome(&outcome),
                no_win_pose,
            },
        ));
        events.push(PlayEvent::Battle {
            battle: request.battle,
            outcome,
        });
        self.start_pending(&mut events);
        events
    }
}

impl PlayGame {
    /// Sets the story counter (`Global` UInt16 0) and the discovery bits
    /// (`gEventGlobal(101)`) the world map reads for its block forms.
    pub fn set_story(&mut self, scenario: i32, discovered: i32, visited: u64) {
        use tantalus_legacy::VariableType;
        let mut worlds = vec![&mut self.world];
        if let Some(session) = self.session.as_mut() {
            worlds.push(session.director.world_mut());
        }
        for world in worlds {
            world.global_memory.set(0, VariableType::UInt16, scenario);
            world.global_memory.set(101, VariableType::Byte, discovered);
            for (word, index) in [92u16, 94, 96, 98].into_iter().enumerate() {
                let bits = (visited >> (16 * word)) as u16;
                world
                    .global_memory
                    .set(index, VariableType::UInt16, i32::from(bits));
            }
        }
    }

    /// Puts the player at `position` on the next field tick (as a load does).
    pub fn place_player(&mut self, position: [f32; 3]) {
        self.pending_position = Some(position);
    }

    /// Writes a script global (`gEventGlobal`) in the running field and the
    /// world between fields, to start a probe at a chosen story state.
    pub fn set_global(&mut self, index: u16, kind: tantalus_legacy::VariableType, value: i32) {
        let mut worlds = vec![&mut self.world];
        if let Some(session) = self.session.as_mut() {
            worlds.push(session.director.world_mut());
        }
        for world in worlds {
            world.global_memory.set(index, kind, value);
        }
    }

    /// Sets the chocobo kind the player owns (`gEventGlobal` 191, 0 none,
    /// 1 to 5 yellow to gold), which makes the world script create its
    /// chocobo.
    pub fn set_chocobo_kind(&mut self, kind: i32) {
        let mut worlds = vec![&mut self.world];
        if let Some(session) = self.session.as_mut() {
            worlds.push(session.director.world_mut());
        }
        for world in worlds {
            world
                .global_memory
                .set(191, tantalus_legacy::VariableType::Byte, kind);
        }
    }

    /// The overworld assets of the disc the story point calls for, loaded
    /// on first use and again when the story crosses to the other disc.
    pub fn world_assets(&mut self) -> Result<std::sync::Arc<WorldAssets>, String> {
        // `WorldConfiguration.GetDisc`: the story counter (`Global` UInt16
        // 0, `w_frameScenePtr`) from 11090 on picks disc 4, else disc 1;
        // the world has only those two (`WMWorld.SetDisc`).
        let scenario = self
            .session
            .as_ref()
            .map(|session| session.director.world())
            .unwrap_or(&self.world)
            .global_memory
            .get(0, tantalus_legacy::VariableType::UInt16)
            .unwrap_or(0);
        let disc = if scenario >= 11090 { 4 } else { 1 };
        if let Some(assets) = self.world_assets.as_ref().filter(|a| a.disc() == disc) {
            return Ok(assets.clone());
        }
        let assets = std::sync::Arc::new(WorldAssets::load(&self.data.streaming, disc)?);
        self.world_assets = Some(assets.clone());
        Ok(assets)
    }

    /// Starts walking the world map at world `(x, z)`.
    pub fn begin_world(&mut self, x: f32, z: f32) -> Result<(), String> {
        self.begin_world_at(Some((x, z)))
    }

    /// Starts walking the world map where the world script put the
    /// controlled character: the saved position, or the new-game default
    /// when none was saved (entry 11 of `EVT_WORLD_WORLD00`).
    pub fn resume_world(&mut self) -> Result<(), String> {
        self.begin_world_at(None)
    }

    fn begin_world_at(&mut self, at: Option<(f32, f32)>) -> Result<(), String> {
        let assets = self.world_assets()?;
        let name = self
            .data
            .db
            .get(&self.world_script_map)
            .cloned()
            .ok_or("no world script in the event db")?;
        let places = crate::world_places::event_places(&self.data.bundle, &name)?;
        let exit = WorldExit::load(&self.data.bundle, &name)?;
        let seed = 0x9E37_79B9_u32 ^ (self.ticks as u32).wrapping_mul(2_654_435_761);
        let globals = self
            .session
            .as_ref()
            .map(|session| session.director.world().global_memory.clone())
            .unwrap_or_else(|| self.world.global_memory.clone());
        let scenario = globals
            .get(0, tantalus_legacy::VariableType::UInt16)
            .unwrap_or(0);
        let discovered = globals
            .get(101, tantalus_legacy::VariableType::Byte)
            .unwrap_or(0);
        let visited = [92u16, 94, 96, 98]
            .iter()
            .enumerate()
            .fold(0u64, |bits, (word, index)| {
                let value = globals
                    .get(*index, tantalus_legacy::VariableType::UInt16)
                    .unwrap_or(0);
                bits | (u64::from(value as u16) << (16 * word))
            });
        // The world script runs beside the walk and picks the place under
        // the actor from the story state, as the original does. A few
        // frames first, so its objects have their positions.
        let mut script_world = self.world.clone();
        // The scripts read the atan table (`AngleToObject`: the moogle turns to the player).
        script_world.copy_providers_from(&self.data.new_game);
        script_world.global_memory = globals.clone();
        let mut script = crate::world_script::WorldScript::start(
            &self.data.bundle,
            &name,
            self.world_script_map,
            script_world,
        )
        .ok();
        if let Some(script) = script.as_mut() {
            for _ in 0..crate::world_script::START_FRAMES {
                if script.tick(0).is_err() {
                    break;
                }
            }
        }
        let (x, z) = match (at, script.as_ref().and_then(|s| s.zidane_start())) {
            (Some(at), _) => at,
            (None, Some((pos, _))) => (pos[0], pos[2]),
            (None, None) => return Err("the world script gave no start position".to_owned()),
        };
        let mut play = WorldPlay::new(assets, places, x, z, seed)?;
        play.set_progress(scenario, visited);
        if let Some(script) = script {
            play.set_script(script);
        }
        if at.is_none()
            && let Some((_, _, yaw)) = play.script_start().and_then(|start| start.zidane)
        {
            play.walker.rot_y = yaw;
            play.walker.rot_true = yaw;
        }
        play.set_forms(&crate::world_data::alternate_form_blocks(
            scenario, discovered,
        ));
        // `w_frameMapConstructor` -> `w_movementChange` (`ff9.cs:5665`): the ride is
        // gEventGlobal 190, so a save made on a chocobo or in an airship loads riding it.
        let ride = globals
            .get(190, tantalus_legacy::VariableType::Byte)
            .unwrap_or(0);
        if let Ok(ride) = u8::try_from(ride)
            && (1..=9).contains(&ride)
        {
            play.set_ride(ride);
        }
        // The world map's main menu has no Save; only its moogle saves.
        if let Some(session) = self.session.as_mut() {
            session.director.world_mut().save_point = false;
        }
        self.world_play = Some(play);
        self.world_exit = Some(exit);
        self.world_song_ride = None;
        self.request_world_song();
        Ok(())
    }

    /// Starts walking the world map from the place that leads back to the
    /// field just left (the reverse of the world script's place switch).
    pub fn start_world(
        &mut self,
        from: Option<i32>,
        events: &mut Vec<PlayEvent>,
    ) -> Result<(), String> {
        let name = self
            .data
            .db
            .get(&self.world_script_map)
            .cloned()
            .ok_or("no world script in the event db")?;
        let places = crate::world_places::event_place_options(&self.data.bundle, &name)?;
        let exit = WorldExit::load(&self.data.bundle, &name)?;
        let from = from.or(self.map());
        let world = self
            .session
            .as_ref()
            .map(|session| session.director.world().clone())
            .unwrap_or_else(|| self.world.clone());
        let mut arrival = None;
        for (code, options) in &places {
            // Places whose exit needs state this world lacks are skipped.
            let leads_back = options.iter().any(|place| {
                let mut probe = world.clone();
                exit.run(*place, &mut probe).ok().flatten() == from
            });
            if leads_back {
                arrival = Some(*code);
                break;
            }
        }
        let code = arrival
            .or_else(|| places.keys().next().copied())
            .ok_or("the world script has no places")?;
        let place = places[&code].first().copied().unwrap_or_default();
        let (cell_x, cell_z) = (i32::from((code & 0xFC) >> 2), i32::from((code >> 8) & 0x3F));
        self.begin_world(cell_x as f32 * 32.0 + 16.0, -(cell_z as f32 * 32.0 + 16.0))?;
        events.push(PlayEvent::WorldWalk { place });
        Ok(())
    }

    /// One frame of walking the world map.
    fn tick_world(
        &mut self,
        held: i32,
        pressed: i32,
        events: &mut Vec<PlayEvent>,
    ) -> Result<(), String> {
        let Some(play) = self.world_play.as_mut() else {
            return Ok(());
        };
        if let Some(screen) = self.chocograph_screen.as_mut() {
            // The list takes the pad while it is open (`ChocographUI`).
            use crate::chocograph_menu::ChocographEvent;
            let event = screen.press(pressed);
            play.hintmap = screen.book.hintmap_id;
            let sound = match event {
                ChocographEvent::Moved => Some(menu_sound::CURSOR),
                ChocographEvent::Confirmed => Some(menu_sound::CONFIRM),
                ChocographEvent::Cancelled | ChocographEvent::Closed => Some(menu_sound::CANCEL),
                ChocographEvent::Refused => Some(menu_sound::REFUSE),
                ChocographEvent::Nothing => None,
            };
            if let Some(id) = sound {
                self.audio.push((self.ticks, AudioCue::Effect { id }));
            }
            if event == ChocographEvent::Closed {
                self.chocograph_screen = None;
                play.chocograph_close();
            }
            return Ok(());
        }
        let mut dismounted = false;
        if pressed & pad::SQUARE != 0 {
            // Square gets off whatever the player rides, or else boards a ship in reach;
            // on foot with none near, the moogle answers it.
            if play.ride != 0 {
                dismounted = true;
                if !play.get_off() {
                    self.audio.push((
                        self.ticks,
                        AudioCue::Effect {
                            id: menu_sound::REFUSE,
                        },
                    ));
                }
            } else {
                // On foot, anything but a ship in reach leaves Square to the world script
                // (`EVT_WORLD_WORLD00` entry 3: `KeyOn(Special)` with ride 0 sets map var 30,
                // entry 2 then requests the moogle, entry 6 function 25).
                play.board();
            }
        }
        // A press that got off the ride is not the moogle's call: `KeyOn(Special)` in the
        // world script reads it with the ride at 0 once the walker is on foot.
        let (held, pressed) = if dismounted {
            (held & !pad::SQUARE, pressed & !pad::SQUARE)
        } else {
            (held, pressed)
        };
        // The script's inventory and Chocograph state follow the player's
        // (a dig's treasure reaches the bag, the save keeps the pick).
        if let Some(session) = self.session.as_ref() {
            play.sync_into_script(session.director.world());
        }
        // A window the world script opened (the moogle's): the choice cursor moves with
        // up and down and a confirm answers it (`DialogBoxSymbols.SetupChoose`).
        let open = play.script_dialogs();
        if open
            .iter()
            .any(|(_, text)| self.choice_texts.contains(text))
        {
            let before = self.cursor;
            if pressed & pad::UP != 0 {
                self.cursor = (self.cursor - 1).max(0);
            }
            if pressed & pad::DOWN != 0 {
                self.cursor += 1;
            }
            if self.cursor != before {
                self.audio.push((
                    self.ticks,
                    AudioCue::Effect {
                        id: menu_sound::CURSOR,
                    },
                ));
            }
            if pressed & CONFIRM != 0 {
                play.set_dialog_choice(self.cursor);
                self.audio.push((
                    self.ticks,
                    AudioCue::Effect {
                        id: menu_sound::CONFIRM,
                    },
                ));
            }
        }
        if !open.is_empty() && pressed & CONFIRM != 0 {
            self.cursor = 0;
        }
        let tick = play.tick(held, pressed, CONFIRM, false);
        if let Some(session) = self.session.as_mut() {
            play.sync_out_of_script(session.director.world_mut());
            // The field's dialog drawing reads the session world's windows.
            play.mirror_windows_into(session.director.world_mut());
            // `OpenMenu(4, 0)` from the moogle: the save screen opens over the world map.
            if play.take_save_request() {
                session.director.world_mut().save_point = true;
                session.director.world_mut().save_requested = true;
            }
        }
        if self.world_song_ride != self.world_play.as_ref().map(|play| play.ride) {
            self.request_world_song();
        }
        let Some(play) = self.world_play.as_mut() else {
            return Ok(());
        };
        if let Some(book) = play.chocograph.clone() {
            // `OpenMenu 5` from the script: the list opens over the world.
            let texts = self.data.menu.chocograph_text.clone();
            self.chocograph_screen =
                Some(crate::chocograph_menu::ChocographScreen::new(book, texts));
        }
        if let Some(field) = tick.field {
            // The world script changed field by itself (its own prompt).
            let session = self.session.take().expect("session");
            self.world = session.director.world().clone();
            self.world_play = None;
            self.enter(field, events)?;
            return Ok(());
        }
        if let Some(scene) = tick.battle.or(tick.script_battle) {
            let request = tantalus_game::field::EncounterRequest {
                rush: 0,
                group: None,
                battle: scene,
                steiner_state: false,
                timer: None,
                pattern_roll: 0,
            };
            if self.live_battles && self.data.battles.is_some() {
                self.pending = Some(PendingBattles {
                    requests: vec![request],
                    fight: None,
                    next_field: None,
                    world_map: None,
                    stage: None,
                });
                self.start_pending(events);
                return Ok(());
            }
            let party = self.party();
            let session = self.session.as_mut().expect("session");
            let outcome = match &self.data.battles {
                Some(battles) => {
                    let items = session.director.world().items.clone();
                    let mut report = battles.fight_with_items(&request, &party, &items);
                    save_party(&report, session.director.world_mut());
                    pay_rewards(&report, session.director.world_mut());
                    self.last_battle = report.stage.take();
                    report
                        .outcome
                        .or_else(|| report.error.clone())
                        .unwrap_or_else(|| "no result".to_owned())
                }
                None => "skipped".to_owned(),
            };
            events.push(PlayEvent::Battle {
                battle: scene,
                outcome,
            });
        }
        if let Some(place) = tick.enter {
            let session = self.session.as_mut().expect("session");
            let mut world = session.director.world().clone();
            let exit = self.world_exit.as_ref().ok_or("no world exit")?;
            let next = exit.run(place, &mut world)?;
            events.push(PlayEvent::WorldMap {
                world_map: 9000,
                location: Some(place),
                next,
            });
            if let Some(field) = next {
                session.director.world_mut().clone_from(&world);
                let session = self.session.take().expect("session");
                self.world = session.director.world().clone();
                self.world_play = None;
                self.enter(field, events)?;
            }
        }
        Ok(())
    }
}

/// What a live battle paid (`BattleResultUI`): EXP per member with the
/// level before and after, AP, gil, items, Steal's take and a card.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BattleResults {
    pub outcome: String,
    /// The party in slot order (`FF9.party.member`), the result screen's rows.
    pub party: Vec<i32>,
    /// `(character, abilities)` learned from this battle's AP (`BattleResultUI.ApLearned`).
    pub learned: Vec<(i32, Vec<i32>)>,
    pub gil: u32,
    pub ap: u16,
    /// `(character, EXP, level before, level after)`.
    pub exp: Vec<(i32, u32, usize, usize)>,
    /// `(item, count)` dropped.
    pub items: Vec<(u32, u32)>,
    pub stolen: Vec<u32>,
    pub card: Option<u32>,
}

fn player_position(world: &FieldWorld) -> Option<[f32; 3]> {
    let player = world.player()?;
    Some(world.actors.get(&player)?.position)
}

/// The longest piece of a pad step that is clamped at once: a run moves 60
/// per tick, and a longer jump could land on another floor past a wall.
const WALK_PIECE: f32 = 15.0;

/// Clamps a pad step to the walkmesh in pieces of at most [`WALK_PIECE`],
/// as the reference checks each of a run's two walk steps; other moves
/// (scripts) clamp once.
fn walk_on_walkmesh(world: &mut FieldWorld, before: Option<[f32; 3]>, floor: &mut Option<i16>) {
    let walked = world
        .player()
        .and_then(|player| world.actors.get(&player))
        .is_some_and(|actor| actor.input_walking);
    let (Some(start), Some(after)) = (before, player_position(world)) else {
        clamp_to_walkmesh(world, before, floor);
        return;
    };
    let delta = [after[0] - start[0], after[2] - start[2]];
    let pieces = (delta[0].hypot(delta[1]) / WALK_PIECE).ceil();
    if !walked || pieces <= 1.0 {
        clamp_to_walkmesh(world, before, floor);
        return;
    }
    let Some(player) = world.player() else {
        return;
    };
    let mut at = start;
    for _ in 0..pieces as usize {
        let next = [
            at[0] + delta[0] / pieces,
            after[1],
            at[2] + delta[1] / pieces,
        ];
        if let Some(actor) = world.actors.get_mut(&player) {
            actor.position = next;
        }
        clamp_to_walkmesh(world, Some(at), floor);
        at = player_position(world).unwrap_or(next);
    }
}

/// Input walking has no collision of its own: keep the player on the
/// walkmesh. A step that lands inside a triangle of the floor the player
/// is on (or, failing that, of another floor, where floors meet at a
/// doorway) is kept and that floor becomes current; a step off every floor
/// slides to the nearest edge point of the current floor.
fn clamp_to_walkmesh(world: &mut FieldWorld, before: Option<[f32; 3]>, floor: &mut Option<i16>) {
    let Some(after) = player_position(world) else {
        return;
    };
    if before == Some(after) || !world.user_control {
        return;
    }
    // Triangles the field scripts switched off are not walkable (`BGI_floorSetActive`).
    let Some(walkable) = world
        .walkable_navigation()
        .map(std::borrow::Cow::into_owned)
    else {
        return;
    };
    let mesh = &walkable;
    let Ok(navigation) = tantalus_game::navigation::FieldNavigation::new(mesh) else {
        return;
    };
    let mut floors: Vec<i16> = mesh
        .triangles
        .iter()
        .map(|triangle| triangle.floor)
        .collect();
    floors.sort_unstable();
    floors.dedup();
    let current = floor
        .or_else(|| {
            let start = before?;
            let point = Point3 {
                x: start[0].round() as i32,
                y: 0,
                z: start[2].round() as i32,
            };
            floors.iter().copied().find(|&candidate| {
                navigation
                    .project_on_floor(point, candidate)
                    .is_ok_and(|p| p.position.x == point.x && p.position.z == point.z)
            })
        })
        .unwrap_or(mesh.active_floor);
    let point = Point3 {
        x: after[0].round() as i32,
        y: 0,
        z: after[2].round() as i32,
    };
    let inside = std::iter::once(current)
        .chain(floors.iter().copied().filter(|&other| other != current))
        .find(|&candidate| {
            navigation
                .project_on_floor(point, candidate)
                .is_ok_and(|p| p.position.x == point.x && p.position.z == point.z)
        });
    let (x, z) = match inside {
        Some(found) => {
            *floor = Some(found);
            (point.x, point.z)
        }
        None => {
            *floor = Some(current);
            match navigation.project_on_floor(point, current) {
                Ok(projection) => (projection.position.x, projection.position.z),
                Err(_) => return,
            }
        }
    };
    if let Some(player) = world.player()
        && let Some(actor) = world.actors.get_mut(&player)
    {
        // Height stays as the scripts set it.
        actor.position[0] = x as f32;
        actor.position[2] = z as f32;
    }
}
