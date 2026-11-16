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
    FieldText, load_navigation, load_tile_animations, parse_background_map, parse_event_db,
};
use crate::field_menu::{FieldMenu, MenuAction};
use crate::field_session::{
    CONFIRM, FieldSession, WorldExit, decode_pool, load_event, prepare_field_world, world_exit,
};
use crate::play_audio::{AudioCue, menu_sound};
use crate::play_save::{self, PlaySave, SAVE_FORMAT, SLOTS};
use crate::world_data::WorldAssets;
use crate::world_play::WorldPlay;
use tantalus_game::battle::BattleCommand;

/// `EventInput` pad bits (Memoria `Global/Event/EventInput.cs`).
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
    game_data: PathBuf,
    battles: Option<FieldBattles>,
    battle_base: BTreeMap<i32, BaseStats>,
    new_game: FieldWorld,
    menu: crate::field_menu::MenuTables,
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

impl PlayData {
    pub fn load(paths: &PlayPaths) -> Result<Self, String> {
        let sa = &paths.streaming_assets;
        let memoria = &paths.memoria;
        let bundle = sa.join("p0data7.bin");
        let field_bundles = (11..=19)
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
        let characters = memoria.join("Memoria.Patcher/StreamingAssets/Data/Characters");
        let base = tantalus_game::new_game::parse_base_stats(&read_text(
            &characters.join("BaseStats.csv"),
        )?)?;
        let levels =
            tantalus_game::new_game::parse_leveling(&read_text(&characters.join("Leveling.csv"))?)?;
        let atan = std::fs::read(&paths.atan_table)
            .map_err(|error| format!("cannot read {}: {error}", paths.atan_table.display()))?;
        let mut new_game = new_game_world(&base, &levels, atan);
        let menu = crate::field_menu::MenuTables::load(
            memoria,
            base.clone(),
            levels.iter().map(|row| row.experience).collect(),
            item_names.clone(),
            text.names("ability/aa_name").unwrap_or_default(),
            text.names("ability/sa_name").unwrap_or_default(),
        )?;
        menu.seed_equipment(&mut new_game);
        new_game.items = tantalus_game::new_game::parse_initial_items(&read_text(
            &characters.join("../Items/InitialItems.csv"),
        )?)?;
        let battles = match &paths.battles {
            Some(ids) => Some(FieldBattles::load(
                sa,
                &bundle,
                ids,
                crate::field_battle::Equipment::load(&characters)?,
            )?),
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
            game_data: paths.game_data.clone(),
            battles,
            battle_base: base,
            new_game,
            menu,
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
pub(crate) fn character_name(id: i32) -> &'static str {
    match id {
        1 => "Vivi",
        2 => "Dagger",
        3 => "Steiner",
        4 => "Freya",
        5 => "Quina",
        6 => "Eiko",
        7 => "Amarant",
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
    /// The open main menu; the field does not run while it is open.
    menu: Option<FieldMenu>,
    save_dir: PathBuf,
    /// Where to put the player after a load, once their actor exists.
    pending_position: Option<[f32; 3]>,
    /// Fight battles one battle tick at a time with [`Self::battle_tick`]
    /// (a player's window) instead of to the end inside the field tick
    /// that starts them. Off by default: the headless paths are unchanged.
    pub live_battles: bool,
    /// The battles a field tick started, while [`Self::live_battles`]
    /// fights them; the field waits until they are over.
    pending: Option<PendingBattles>,
    /// Sound requests made since the last [`PlayGame::take_audio`], with the tick of each.
    /// Runtime-only: the run never reads them back.
    audio: Vec<(u64, AudioCue)>,
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
        let world = data.new_game.clone();
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
            world_assets: None,
            world_exit: None,
            stopped: None,
            ticks: 0,
            floor: None,
            last_battle: None,
            menu: None,
            save_dir: play_save::default_dir(),
            pending_position: None,
            live_battles: false,
            pending: None,
            audio: Vec::new(),
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

    /// The live world (the current field's, or the one between fields).
    pub fn world(&self) -> &FieldWorld {
        match &self.session {
            Some(session) => session.director.world(),
            None => &self.world,
        }
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
                        text(character_name(keyword_character(keyword)).to_owned())
                    }
                    // `[PTYn]`: the character in party slot n (1-based).
                    "Party" => {
                        let slot = param(0).unwrap_or(1).max(1) as usize - 1;
                        let id = world.party.get(slot).copied().unwrap_or(0);
                        text(character_name(id).to_owned())
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
        match self.data.backgrounds.get(&map) {
            Some(background) => match load_tile_animations(&self.data.field_bundles, background) {
                Ok((animations, cameras)) => world.provide_tile_animations(animations, cameras),
                Err(_) => world.clear_tile_animations(),
            },
            None => world.clear_tile_animations(),
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
                    globals: PartyGlobals::from_globals(world.global_memory.bytes()),
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
        if let Some(mut menu) = self.menu.take() {
            let mut open = true;
            if pressed != 0 {
                let tables = &self.data.menu;
                let world = match self.session.as_mut() {
                    Some(session) => session.director.world_mut(),
                    None => &mut self.world,
                };
                let before = menu.revision;
                open = menu.press(pressed, world, tables);
                // `FF9SFX_Play(103)` moves the cursor, `101` confirms, `102` cancels or refuses.
                let sound = if pressed & CONFIRM != 0 {
                    Some(menu_sound::CONFIRM)
                } else if pressed & (pad::CANCEL | pad::CIRCLE) != 0 {
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
                menu.slot_labels = self.slot_labels();
                menu.slots_stale = false;
                menu.revision += 1;
            }
            if open {
                self.menu = Some(menu);
            }
            return Ok(true);
        }
        let Some(session) = self.session.as_mut() else {
            return Ok(false);
        };
        let world = session.director.world_mut();
        let allowed =
            world.user_control && !world.menu_disabled && !world.windows.values().any(|open| *open);
        if world.save_requested && allowed {
            // A moogle asked for the save screen.
            world.save_requested = false;
            let mut menu = FieldMenu::save_screen();
            menu.slot_labels = self.slot_labels();
            menu.slots_stale = false;
            self.menu = Some(menu);
            return Ok(true);
        }
        if pressed & pad::START != 0 && allowed && held & pad::START != 0 {
            self.menu = Some(FieldMenu::new());
            return Ok(true);
        }
        Ok(false)
    }

    /// Folder the save slots live in.
    pub fn set_save_dir(&mut self, dir: PathBuf) {
        self.save_dir = dir;
    }

    /// Writes the game to `slot` (1-based): the map, the player's position
    /// and the world.
    pub fn save_slot(&mut self, slot: usize) -> Result<(), String> {
        let map = self.map().ok_or("there is no field to save")?;
        let world = self.world();
        let save = PlaySave {
            format: SAVE_FORMAT,
            map,
            position: player_position(world),
            ticks: self.ticks,
            world: world.for_save(),
        };
        play_save::write(&self.save_dir, slot, &save)
    }

    /// Replaces the game with `slot`: enters the saved map with the saved
    /// world and puts the player back.
    pub fn load_slot(&mut self, slot: usize, events: &mut Vec<PlayEvent>) -> Result<(), String> {
        let save = play_save::read(&self.save_dir, slot)?.ok_or("that slot is empty")?;
        let mut world = save.world;
        world.copy_providers_from(&self.data.new_game);
        self.world = world;
        self.session = None;
        self.menu = None;
        self.stopped = None;
        self.ticks = save.ticks;
        self.pending_position = save.position;
        self.enter(save.map, events)
    }

    /// Opens the load screen (a play window started without a field to
    /// resume).
    pub fn open_load_screen(&mut self) {
        let mut menu = FieldMenu::load_screen();
        menu.slot_labels = self.slot_labels();
        menu.slots_stale = false;
        self.menu = Some(menu);
    }

    /// One line per save slot for the save and load screens.
    pub fn slot_labels(&self) -> Vec<String> {
        (1..=SLOTS)
            .map(|slot| match play_save::read(&self.save_dir, slot) {
                Ok(Some(save)) => {
                    let tables = &self.data.menu;
                    let lead = save.world.party.first().copied();
                    let who = lead.map_or(String::new(), |id| {
                        let level =
                            tables.level(save.world.character_exp.get(&id).copied().unwrap_or(0));
                        format!("{} Lv {level}  ", character_name(id))
                    });
                    let place = self.data.field_name(save.map).unwrap_or("?");
                    format!("{slot}  {who}{} Gil  {place}", save.world.gil)
                }
                Ok(None) => format!("{slot}  Empty"),
                Err(_) => format!("{slot}  Unreadable"),
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

    /// Drains the sound requests made so far (script opcodes, field changes, battles, menu and
    /// choice presses), each with the game tick it happened on.
    pub fn take_audio(&mut self) -> Vec<(u64, AudioCue)> {
        std::mem::take(&mut self.audio)
    }

    /// The main menu to draw, with a number that changes when it does.
    pub fn menu_frame(&self) -> Option<(u64, crate::field_menu::MenuFrame)> {
        let menu = self.menu.as_ref()?;
        Some((menu.revision, menu.frame(self.world(), &self.data.menu)))
    }

    /// One field tick (30 a second) with `held` pad bits.
    pub fn tick(&mut self, held: i32) -> Result<Vec<PlayEvent>, String> {
        let mut events = Vec::new();
        if self.stopped.is_some() || self.pending.is_some() {
            return Ok(events);
        }
        let pressed = held & !self.previous;
        self.previous = held;
        self.ticks += 1;
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
        // A confirm press lasts one tick; holding it does not repeat.
        let mut input = held & !CONFIRM;
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
        if pressed & CONFIRM != 0 {
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
        for event in session.director.world_mut().take_sound_events() {
            self.audio.push((self.ticks, AudioCue::Script(event)));
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
                    },
                ));
                let outcome = match &self.data.battles {
                    Some(battles) => {
                        let items = session.director.world().items.clone();
                        let mut report = battles.fight_with_items(request, &party, &items);
                        save_party(&report, &mut session.director.world_mut().character_stats);
                        pay_rewards(&report, session.director.world_mut());
                        stages.push(report.stage.take());
                        report.outcome.unwrap_or_else(|| "no result".to_owned())
                    }
                    None => "skipped".to_owned(),
                };
                self.audio.push((
                    self.ticks,
                    AudioCue::BattleEnded {
                        end: crate::play_audio::BattleEnd::from_outcome(&outcome),
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

        self.leave(step.next_field, step.world_map, &mut events)?;
        Ok(events)
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
    pub fn battle(&self) -> Option<&LiveFight> {
        self.pending.as_ref()?.fight.as_ref()
    }

    /// Name of item `id` from the game text.
    pub fn item_name(&self, id: i32) -> Option<&str> {
        self.data.item_names.get(&id).map(String::as_str)
    }

    /// Starts the first pending battle that sets up; one that cannot start
    /// is reported and skipped. With none left, the field resumes.
    fn start_pending(&mut self, events: &mut Vec<PlayEvent>) {
        let party = self.party();
        let map = self.map().unwrap_or(0);
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
                    pending.fight = Some(fight);
                    self.audio.push((
                        self.ticks,
                        AudioCue::BattleStarted {
                            map,
                            battle: request.battle,
                        },
                    ));
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
        self.audio.push((self.ticks, AudioCue::BattleLeft));
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
        if fight.step(FightControl::Player(command)) {
            return events;
        }
        let pending = self.pending.as_mut().expect("pending");
        let fight = pending.fight.take().expect("fight");
        let request = pending.requests.remove(0);
        let battles = self.data.battles.as_ref().expect("battle data");
        let mut report = fight.finish(battles);
        let session = self.session.as_mut().expect("session");
        save_party(&report, &mut session.director.world_mut().character_stats);
        pay_rewards(&report, session.director.world_mut());
        if let Some(stage) = report.stage.take() {
            pending.stage = Some(stage);
        }
        let outcome = report.outcome.unwrap_or_else(|| "no result".to_owned());
        self.audio.push((
            self.ticks,
            AudioCue::BattleEnded {
                end: crate::play_audio::BattleEnd::from_outcome(&outcome),
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
    /// The overworld assets, loaded on first use.
    pub fn world_assets(&mut self) -> Result<std::sync::Arc<WorldAssets>, String> {
        if let Some(assets) = &self.world_assets {
            return Ok(assets.clone());
        }
        let assets = std::sync::Arc::new(WorldAssets::load(&self.data.streaming, 1)?);
        self.world_assets = Some(assets.clone());
        Ok(assets)
    }

    /// Starts walking the world map at world `(x, z)`.
    pub fn begin_world(&mut self, x: f32, z: f32) -> Result<(), String> {
        let assets = self.world_assets()?;
        let name = self
            .data
            .db
            .get(&9000)
            .cloned()
            .ok_or("no world script in the event db")?;
        let places = crate::world_places::event_places(&self.data.bundle, &name)?;
        let exit = WorldExit::load(&self.data.bundle, &name)?;
        let seed = 0x9E37_79B9_u32 ^ (self.ticks as u32).wrapping_mul(2_654_435_761);
        self.world_play = Some(WorldPlay::new(assets, places, x, z, seed)?);
        self.world_exit = Some(exit);
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
            .get(&9000)
            .cloned()
            .ok_or("no world script in the event db")?;
        let places = crate::world_places::event_places(&self.data.bundle, &name)?;
        let exit = WorldExit::load(&self.data.bundle, &name)?;
        let from = from.or(self.map());
        let world = self
            .session
            .as_ref()
            .map(|session| session.director.world().clone())
            .unwrap_or_else(|| self.world.clone());
        let mut arrival = None;
        for (code, place) in &places {
            let mut probe = world.clone();
            // Places whose exit needs state this world lacks are skipped.
            if exit.run(*place, &mut probe).ok().flatten() == from {
                arrival = Some(*code);
                break;
            }
        }
        let code = arrival
            .or_else(|| places.keys().next().copied())
            .ok_or("the world script has no places")?;
        let place = places[&code];
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
        let tick = play.tick(held, pressed, CONFIRM, false);
        if let Some(scene) = tick.battle {
            let request = tantalus_game::field::EncounterRequest {
                rush: 0,
                group: None,
                battle: scene,
                steiner_state: false,
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
                    save_party(&report, &mut session.director.world_mut().character_stats);
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
    let Some(mesh) = world.navigation() else {
        return;
    };
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
