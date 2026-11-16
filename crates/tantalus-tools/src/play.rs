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

use crate::field_battle::{FieldBattles, PartyFighter, PartyGlobals, pay_rewards, save_party};
use crate::field_data::{
    FieldText, load_navigation, load_tile_animations, parse_background_map, parse_event_db,
};
use crate::field_session::{
    CONFIRM, FieldSession, decode_pool, load_event, prepare_field_world, world_exit,
};

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
    bundle: PathBuf,
    field_bundles: Vec<PathBuf>,
    db: BTreeMap<i32, String>,
    backgrounds: BTreeMap<i32, String>,
    text: FieldText,
    battles: Option<FieldBattles>,
    battle_base: BTreeMap<i32, BaseStats>,
    new_game: FieldWorld,
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
        let characters = memoria.join("Memoria.Patcher/StreamingAssets/Data/Characters");
        let base = tantalus_game::new_game::parse_base_stats(&read_text(
            &characters.join("BaseStats.csv"),
        )?)?;
        let levels =
            tantalus_game::new_game::parse_leveling(&read_text(&characters.join("Leveling.csv"))?)?;
        let atan = std::fs::read(&paths.atan_table)
            .map_err(|error| format!("cannot read {}: {error}", paths.atan_table.display()))?;
        let mut new_game = new_game_world(&base, &levels, atan);
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
            bundle,
            field_bundles,
            db,
            backgrounds,
            text,
            battles,
            battle_base: base,
            new_game,
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
    /// The run cannot go on.
    Stopped(String),
}

/// The run.
pub struct PlayGame {
    data: PlayData,
    session: Option<FieldSession>,
    world: FieldWorld,
    choice_texts: BTreeSet<u16>,
    plain: BTreeMap<u16, String>,
    layouts: BTreeMap<u16, crate::field_data::TextLayout>,
    shown: BTreeMap<u8, u16>,
    previous: i32,
    /// Answer highlighted in an open choice window.
    pub cursor: i32,
    /// World map locations to leave through, used in order.
    pub world_exits: Vec<i32>,
    pub stopped: Option<String>,
    pub ticks: u64,
    /// Walkmesh floor the player stands on (found on the first step).
    floor: Option<i16>,
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
            shown: BTreeMap::new(),
            previous: 0,
            cursor: 0,
            world_exits: Vec::new(),
            stopped: None,
            ticks: 0,
            floor: None,
        };
        let mut events = Vec::new();
        game.enter(start, &mut events)?;
        Ok((game, events))
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

    /// Layout of each open window, in window order, for drawing. The text
    /// has no layout when the field's text failed to load.
    pub fn open_dialogs(&self) -> Vec<(u8, u16, crate::field_data::TextLayout)> {
        let world = self.world();
        world
            .windows
            .iter()
            .filter(|(_, open)| **open)
            .filter_map(|(window, _)| {
                let text = *world.window_texts.get(window)?;
                Some((*window, text, self.layouts.get(&text)?.clone()))
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
            }
            Err(_) => {
                world.dialog_signals.clear();
                self.choice_texts.clear();
                self.plain.clear();
                self.layouts.clear();
            }
        }
        let mut session = FieldSession::start(map, world, pool)?;
        session
            .director
            .set_walk_bits(Some((pad::UP, pad::DOWN, pad::LEFT, pad::RIGHT)));
        session.director.set_walk_hold_bit(Some(pad::CANCEL));
        self.session = Some(session);
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

    /// One field tick (30 a second) with `held` pad bits.
    pub fn tick(&mut self, held: i32) -> Result<Vec<PlayEvent>, String> {
        let mut events = Vec::new();
        if self.stopped.is_some() {
            return Ok(events);
        }
        let pressed = held & !self.previous;
        self.previous = held;
        self.ticks += 1;
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
            if pressed & pad::UP != 0 {
                self.cursor = (self.cursor - 1).max(0);
            }
            if pressed & pad::DOWN != 0 {
                self.cursor += 1;
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
                }
                session.director.world_mut().close_window(*window);
            }
            self.cursor = 0;
        }
        let before = player_position(session.director.world());
        let step = session.tick(input)?;
        clamp_to_walkmesh(session.director.world_mut(), before, &mut self.floor);
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

        if !step.encounters.is_empty() {
            let party = self.party();
            let session = self.session.as_mut().expect("session");
            for request in &step.encounters {
                let outcome = match &self.data.battles {
                    Some(battles) => {
                        let items = session.director.world().items.clone();
                        let report = battles.fight_with_items(request, &party, &items);
                        save_party(&report, &mut session.director.world_mut().character_stats);
                        pay_rewards(&report, session.director.world_mut());
                        report.outcome.unwrap_or_else(|| "no result".to_owned())
                    }
                    None => "skipped".to_owned(),
                };
                events.push(PlayEvent::Battle {
                    battle: request.battle,
                    outcome,
                });
            }
            session.resume_after_battle()?;
        }

        let session = self.session.as_mut().expect("session");
        let mut next = step.next_field;
        if next.is_none()
            && let Some(world_map) = step.world_map
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
                    return self
                        .stop(
                            format!("world map {world_map}: no field to go to"),
                            &mut events,
                        )
                        .map(|()| events);
                }
            }
        }
        if let Some(next) = next {
            let session = self.session.take().expect("session");
            self.world = session.director.world().clone();
            self.enter(next, &mut events)?;
        }
        Ok(events)
    }
}

fn player_position(world: &FieldWorld) -> Option<[f32; 3]> {
    let player = world.player()?;
    Some(world.actors.get(&player)?.position)
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
