//! Steam save files (`SavedData_ww.dat`) in and out of the play window.
//!
//! The container, its encryption and the field layout of one save are
//! `tantalus_legacy::saves` (Memoria `SharedDataBytesStorage`, `AESCryptography`,
//! `JsonParser`). This file maps that state onto [`FieldWorld`] and back:
//!
//! * [`apply`] is the load (`JsonParser.ParseToFF9StateSystem` /
//!   `ParseCommonJsonToData`): party, characters, items, gil, event globals, cards, config.
//! * [`capture`] is the save (`ParseCommonDataToJson` and friends). It starts from the state the
//!   game was loaded from (or a new-game state) and overwrites what the world knows, so the
//!   sections Tantalus does not model (achievements, stat bonuses, reserved buffers) go back to
//!   the file as they came.
//!
//! Slot numbers are the file's own: `slot` 0 to 9 and `save` 0 to 14, and the autosave block.
//!
//! The reference stores no player position or facing (`JsonParser.ParseFromFF9StateSystem` keeps
//! `fldMapNo`, `fldLocNo` and the field entrance in `gEventGlobal`); a load enters the field and
//! its script puts the player at that entrance.

use std::path::Path;

use tantalus_game::FieldWorld;
use tantalus_game::field::settings::GameSettings;
use tantalus_game::field::world::CharacterStats;
use tantalus_legacy::saves::ff9_tree::{CardSlot, EventGlobal, Ff9SaveTree};
use tantalus_legacy::saves::state::{CharacterState, ItemStack, SaveState};
use tantalus_legacy::saves::{
    BASE_SAVE_COUNT_PER_SLOT, BASE_SAVE_SLOT_COUNT, BaseSaveBlockState, BaseSaveMetadata,
    BaseSavePreview, BaseSavePreviewCharacter, LegacySaveImporter, LegacySaveLocation,
};
use tantalus_legacy::{GLOBAL_VARIABLE_BYTES, VariableMemory};

use crate::field_menu::MenuTables;

/// `FF9StateSystem.mode` of a field: `EventEngine.gMode` 1 is the field, 3 the world map.
pub const MODE_FIELD: u8 = 1;
pub const MODE_WORLD: u8 = 3;

/// The message of a slot that holds no save.
pub const EMPTY_SLOT: &str = "that slot is empty";

/// A save read from the file.
#[derive(Debug, Clone)]
pub struct SteamSave {
    pub location: LegacySaveLocation,
    pub metadata: BaseSaveMetadata,
    pub state: SaveState,
}

/// What [`apply`] decided about where the game resumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resume {
    /// `fldMapNo`: the field map to enter.
    pub map: i32,
    /// The save was made on the world map (`wldMapNo`, `wldLocNo`).
    pub world: Option<(i16, i16)>,
}

/// Parses a slot: `auto`, a flat index `N` (`slot * 15 + save`, 0 to 149) or `SLOT:SAVE`.
pub fn parse_location(text: &str) -> Result<LegacySaveLocation, String> {
    let text = text.trim();
    let bad =
        || format!("bad slot `{text}`: want `auto`, a number 0 to 149, or SLOT:SAVE (0-based)");
    if text.eq_ignore_ascii_case("auto") {
        return Ok(LegacySaveLocation {
            is_autosave: true,
            slot_id: -1,
            save_id: -1,
        });
    }
    let (slot, save) = match text.split_once(':') {
        Some((slot, save)) => (
            slot.parse::<usize>().map_err(|_| bad())?,
            save.parse::<usize>().map_err(|_| bad())?,
        ),
        None => {
            let flat = text.parse::<usize>().map_err(|_| bad())?;
            (
                flat / BASE_SAVE_COUNT_PER_SLOT,
                flat % BASE_SAVE_COUNT_PER_SLOT,
            )
        }
    };
    if slot >= BASE_SAVE_SLOT_COUNT || save >= BASE_SAVE_COUNT_PER_SLOT {
        return Err(bad());
    }
    Ok(LegacySaveLocation {
        is_autosave: false,
        slot_id: slot as i32,
        save_id: save as i32,
    })
}

/// The file's latest save (`MetaData.LatestSlot` and `LatestSave`), else the autosave.
pub fn latest_location(bytes: &[u8]) -> Result<LegacySaveLocation, String> {
    let metadata = BaseSaveMetadata::decode(bytes)
        .map_err(|error| format!("save file: {} (at byte {})", error.context, error.offset))?;
    let slot = usize::try_from(metadata.latest_slot).ok();
    let save = usize::try_from(metadata.latest_save).ok();
    match (slot, save) {
        (Some(slot), Some(save))
            if slot < BASE_SAVE_SLOT_COUNT && save < BASE_SAVE_COUNT_PER_SLOT =>
        {
            Ok(LegacySaveLocation {
                is_autosave: false,
                slot_id: slot as i32,
                save_id: save as i32,
            })
        }
        _ => parse_location("auto"),
    }
}

/// Reads one slot of the file at `path` (read-only).
pub fn read_slot(path: &Path, location: &LegacySaveLocation) -> Result<SteamSave, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    read_slot_bytes(&bytes, location)
}

pub fn read_slot_bytes(bytes: &[u8], location: &LegacySaveLocation) -> Result<SteamSave, String> {
    let decoded = LegacySaveImporter::decode_base(bytes, location)
        .map_err(|error| format!("save file: {} (at byte {})", error.context, error.offset))?;
    if decoded.block.state != BaseSaveBlockState::Saved {
        return Err(EMPTY_SLOT.to_owned());
    }
    let state = SaveState::decode(&decoded.block.payload)
        .map_err(|error| format!("save data: {} (at byte {})", error.context, error.offset))?;
    Ok(SteamSave {
        location: location.clone(),
        metadata: decoded.metadata,
        state,
    })
}

/// Every location of a file: the autosave, then 10 slots of 15 saves.
pub fn all_locations() -> Vec<LegacySaveLocation> {
    let mut locations = vec![LegacySaveLocation {
        is_autosave: true,
        slot_id: -1,
        save_id: -1,
    }];
    for slot in 0..BASE_SAVE_SLOT_COUNT {
        for save in 0..BASE_SAVE_COUNT_PER_SLOT {
            locations.push(LegacySaveLocation {
                is_autosave: false,
                slot_id: slot as i32,
                save_id: save as i32,
            });
        }
    }
    locations
}

/// The slots of the file that hold a save, in file order, with a one-line summary each.
pub fn list_saves(bytes: &[u8]) -> Result<Vec<(LegacySaveLocation, String)>, String> {
    let mut found = Vec::new();
    for location in all_locations() {
        match read_slot_bytes(bytes, &location) {
            Ok(save) => {
                let state = &save.state;
                found.push((
                    location,
                    format!(
                        "map {}, scenario {}, {} gil, {} s played",
                        state.field.fld_map_no,
                        state.scenario_counter(),
                        state.party.gil,
                        state.play_time_seconds as u64
                    ),
                ));
            }
            Err(message) if message == EMPTY_SLOT => {}
            Err(message) => return Err(message),
        }
    }
    Ok(found)
}

/// `SelectOldSaveSlot` / `GetPlayerFromOldSave`: the character id the save's `slot`th record
/// holds. Slots 5 to 7 hold Cinna, Marcus and Blank while the record's category has bit 16
/// (a temporary guest), Quina, Eiko and Amarant otherwise; slot 8 is Beatrix.
pub fn slot_character(slot: usize, category: u8) -> Option<i32> {
    let temporary = category & 16 != 0;
    match slot {
        0..=4 => Some(slot as i32),
        5..=7 => Some(if temporary {
            slot as i32 + 3
        } else {
            slot as i32
        }),
        8 => Some(11),
        _ => None,
    }
}

fn game_settings(state: &SaveState, base: GameSettings) -> GameSettings {
    let cfg = &state.carried.setting;
    GameSettings {
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
        ..base
    }
}

/// Puts the saved game into `world` (a new-game world; its runtime providers stay).
pub fn apply(
    state: &SaveState,
    world: &mut FieldWorld,
    tables: &MenuTables,
) -> Result<Resume, String> {
    world.global_memory =
        VariableMemory::from_bytes(state.variables.as_bytes(), GLOBAL_VARIABLE_BYTES)
            .ok_or("gEventGlobal has the wrong size")?;
    world.map_memory.clear();
    world.gil = state.party.gil.min(9_999_999);
    world.items.clear();
    for stack in state.party.items.iter().flatten() {
        if stack.count > 0 {
            *world.items.entry(i32::from(stack.id)).or_insert(0) += i32::from(stack.count);
        }
    }
    world.party = state
        .party
        .members
        .iter()
        .flatten()
        .filter_map(|&slot| {
            let record = state.party.characters.get(usize::from(slot))?;
            slot_character(usize::from(slot), record.category)
        })
        .collect();
    world.back_row.clear();
    for (slot, record) in state.party.characters.iter().enumerate() {
        let Some(id) = slot_character(slot, record.category) else {
            continue;
        };
        world.character_stats.insert(
            id,
            CharacterStats {
                current_hp: i32::from(record.cur.hp),
                max_hp: i32::from(record.max.hp),
                current_mp: i32::from(record.cur.mp),
                max_mp: i32::from(record.max.mp),
            },
        );
        if record.exp > 0 {
            world.character_exp.insert(id, u64::from(record.exp));
        } else {
            world.character_exp.remove(&id);
        }
        let mut equipment = [None; 5];
        for (place, item) in equipment.iter_mut().zip(record.equip) {
            *place = (item != 255).then_some(u16::from(item));
        }
        world.equipment.insert(id, equipment);
        world.trance.insert(id, record.trance);
        // `info.row`: 0 is the back row (`btl_init.SetupBattlePlayerSingle`).
        if record.info.row == 0 {
            world.back_row.insert(id);
        }
        apply_abilities(world, tables, id, record);
    }
    let cards = &state.cards;
    world.cards.clear();
    world.card_records.clear();
    for slot in cards.cards.iter().flatten() {
        world.cards.push(u32::from(slot.id));
        world.card_records.push(tantalus_game::cards::CardRecord {
            id: slot.id,
            atk: slot.atk,
            kind: slot.card_type.clamp(0, 3) as u8,
            pdef: slot.pdef,
            mdef: slot.mdef,
            cpoint: slot.cpoint,
            arrow: slot.arrow,
        });
    }
    world.card_stats = tantalus_game::cards::CardCounts {
        win: cards.win.max(0) as u32,
        lose: cards.lose.max(0) as u32,
        draw: cards.draw.max(0) as u32,
    };
    world.settings = game_settings(state, world.settings.clone());
    world.field_map_no = i32::from(state.field.fld_map_no);
    let on_world_map = state.mode.mode == MODE_WORLD;
    Ok(Resume {
        map: i32::from(state.field.fld_map_no),
        world: on_world_map.then_some((state.world.wld_map_no, state.world.wld_loc_no)),
    })
}

/// `player.pa` and `player.sa`: ability progress by the preset's ability list, and the
/// support abilities switched on (ids 192 + bit).
fn apply_abilities(world: &mut FieldWorld, tables: &MenuTables, id: i32, record: &CharacterState) {
    world.ability_pa.remove(&id);
    world.learned_abilities.remove(&id);
    world.support_enabled.remove(&id);
    if let Some(list) = tables.abilities.learnable.get(&id) {
        for (entry, &points) in list.iter().zip(&record.ability_ap) {
            let points = i32::from(points);
            if points == 0 {
                continue;
            }
            // An ability is learned once its AP reach the goal.
            let points = if entry.max_ap > 0 {
                points.min(entry.max_ap)
            } else {
                points
            };
            world
                .ability_pa
                .entry(id)
                .or_default()
                .insert(entry.id, points);
            if entry.max_ap > 0 && points >= entry.max_ap {
                world
                    .learned_abilities
                    .entry(id)
                    .or_default()
                    .insert(entry.id);
            }
        }
    }
    let enabled: Vec<i32> = (0..64)
        .filter(|bit| record.support_abilities[bit / 32] & (1 << (bit % 32)) != 0)
        .map(|bit| 192 + bit as i32)
        .collect();
    if !enabled.is_empty() {
        world.support_enabled.insert(id, enabled);
    }
}

/// `CharacterSerialNumber` of each character's first outfit.
const DEFAULT_SERIAL: [u8; 12] = [0, 2, 3, 7, 12, 9, 10, 13, 14, 15, 16, 18];

fn default_record(id: i32, slot: usize, tables: &MenuTables) -> CharacterState {
    let defaults = tables
        .abilities
        .defaults
        .get(&id)
        .copied()
        .unwrap_or_default();
    let mut record = SaveState::empty().party.characters[slot].clone();
    record.name = character_title(id).to_owned();
    record.category = defaults.category;
    record.info.slot_no = slot as u8;
    record.info.serial_no = DEFAULT_SERIAL.get(id as usize).copied().unwrap_or(0);
    record.info.row = defaults.row;
    record.info.win_pose = defaults.win_pose;
    record.info.menu_type = defaults.menu_type;
    if let Some(base) = tables.base.get(&id) {
        record.basis.stats = [base.speed, base.strength, base.magic, base.spirit];
    }
    record.ability_ap = vec![0; 48];
    record
}

/// Default names of the 12 characters (`FF9TextTool.CharacterDefaultName`).
fn character_title(id: i32) -> &'static str {
    match id {
        0 => "Zidane",
        1 => "Vivi",
        2 => "Dagger",
        3 => "Steiner",
        4 => "Freya",
        5 => "Quina",
        6 => "Eiko",
        7 => "Amarant",
        8 => "Cinna",
        9 => "Marcus",
        10 => "Blank",
        _ => "Beatrix",
    }
}

/// The state to write for the running game: `base` (what the game was loaded from, or
/// [`SaveState::empty`]) with everything [`apply`] reads replaced by the world's values.
pub fn capture(
    world: &FieldWorld,
    map: i32,
    ticks: u64,
    tables: &MenuTables,
    base: &SaveState,
) -> Result<SaveState, String> {
    let mut state = base.clone();
    state.variables = EventGlobal::from_bytes(world.global_memory.bytes().to_vec())
        .map_err(|error| error.context)?;
    state.field.fld_map_no = i16::try_from(map).map_err(|_| "map number out of range")?;
    state.mode.mode = MODE_FIELD;
    state.play_time_seconds = ticks as f64 / 60.0;
    let party = &mut state.party;
    party.gil = world.gil;
    party.items = vec![None; 256];
    let mut next = 0;
    for (&id, &count) in &world.items {
        if let Ok(id) = u8::try_from(id)
            && id != 255
            && count > 0
            && next < 256
        {
            party.items[next] = Some(ItemStack {
                id,
                count: count.min(99) as u8,
            });
            next += 1;
        }
    }
    // The guest of slots 5 to 7 (Cinna, Marcus, Blank) holds the slot while its permanent
    // character is not around: in the party, or already there in the file.
    let mut records: Vec<CharacterState> = Vec::with_capacity(9);
    for slot in 0..9 {
        let held = party.characters.get(slot).cloned();
        let permanent = slot_character(slot, 0).unwrap_or(0);
        let guest = (5..8).contains(&slot).then_some(slot as i32 + 3);
        let held_is_guest = held
            .as_ref()
            .is_some_and(|record| record.category & 16 != 0);
        let id = match guest {
            Some(guest)
                if world.party.contains(&guest)
                    || (held_is_guest && !world.party.contains(&permanent)) =>
            {
                guest
            }
            _ => permanent,
        };
        // A record the file never described (blank name) starts from the new-game values.
        let mut record = match held {
            Some(record)
                if !record.name.is_empty()
                    && (guest.is_none() || (record.category & 16 != 0) == (id != permanent)) =>
            {
                record
            }
            _ => default_record(id, slot, tables),
        };
        record.info.slot_no = slot as u8;
        update_record(&mut record, world, tables, id);
        records.push(record);
    }
    party.members = [None; 4];
    for (place, &id) in party.members.iter_mut().zip(&world.party) {
        *place = records
            .iter()
            .position(|record| {
                slot_character(usize::from(record.info.slot_no), record.category) == Some(id)
            })
            .map(|slot| slot as u8);
    }
    party.characters = records;
    state.cards.win = world.card_stats.win.min(i16::MAX as u32) as i16;
    state.cards.lose = world.card_stats.lose.min(i16::MAX as u32) as i16;
    state.cards.draw = world.card_stats.draw.min(i16::MAX as u32) as i16;
    state.cards.cards = vec![None; 100];
    for (place, record) in state.cards.cards.iter_mut().zip(&world.card_records) {
        *place = Some(CardSlot {
            id: record.id,
            side: 0,
            atk: record.atk,
            card_type: i32::from(record.kind),
            pdef: record.pdef,
            mdef: record.mdef,
            cpoint: record.cpoint,
            arrow: record.arrow,
        });
    }
    let cfg = &mut state.carried.setting;
    let settings = &world.settings;
    cfg.cursor = settings.cursor;
    cfg.atb = settings.atb;
    cfg.camera = settings.camera;
    cfg.move_ = settings.move_;
    cfg.vibe = settings.vibe;
    cfg.btl_speed = settings.btl_speed;
    cfg.skip_btl_camera = settings.skip_btl_camera;
    cfg.fld_msg = settings.fld_msg;
    cfg.here_icon = settings.here_icon;
    cfg.win_type = settings.win_type;
    Ok(state)
}

fn update_record(record: &mut CharacterState, world: &FieldWorld, tables: &MenuTables, id: i32) {
    let exp = world.character_exp.get(&id).copied().unwrap_or(0);
    record.exp = exp.min(u64::from(u32::MAX)) as u32;
    record.level = tables.level(exp) as u8;
    if let Some(stats) = world.character_stats.get(&id) {
        record.cur.hp = stats.current_hp.clamp(0, 9999) as u16;
        record.max.hp = stats.max_hp.clamp(0, 9999) as u16;
        record.cur.mp = stats.current_mp.clamp(0, 999) as i16;
        record.max.mp = stats.max_mp.clamp(0, 999) as i16;
        // The basis is the level's value before equipment and abilities; a character the file
        // never described gets the current maximum.
        if record.basis.max_hp == 0 {
            record.basis.max_hp = record.max.hp as i16;
            record.basis.max_mp = record.max.mp;
        }
    }
    for (place, item) in record.equip.iter_mut().zip(world.equipped(id)) {
        *place = item.map_or(255, |item| item.min(255) as u8);
    }
    if let Some(&trance) = world.trance.get(&id) {
        record.trance = trance;
    }
    record.info.row = u8::from(!world.back_row.contains(&id));
    if world.party.contains(&id) {
        record.info.party = 1;
    }
    let mut points = vec![0_u8; record.ability_ap.len().max(48)];
    if let Some(list) = tables.abilities.learnable.get(&id) {
        let learned = world.learned_abilities.get(&id);
        let progress = world.ability_pa.get(&id);
        for (index, entry) in list.iter().enumerate().take(points.len()) {
            let value = if learned.is_some_and(|set| set.contains(&entry.id)) {
                entry.max_ap
            } else {
                progress
                    .and_then(|map| map.get(&entry.id))
                    .copied()
                    .unwrap_or(0)
            };
            points[index] = value.clamp(0, 255) as u8;
        }
    }
    record.ability_ap = points;
    let mut support = [0_u32; 2];
    for &ability in world.support_enabled.get(&id).into_iter().flatten() {
        let bit = ability - 192;
        if (0..64).contains(&bit) {
            support[(bit / 32) as usize] |= 1 << (bit % 32);
        }
    }
    record.support_abilities = support;
    if record.name.is_empty() {
        record.name = character_title(id).to_owned();
    }
}

/// `SaveSlotPreview`: the line the save and load screens show.
fn preview(state: &SaveState, place: &str, timestamp: f64) -> BaseSavePreview {
    let mut characters: [Option<BaseSavePreviewCharacter>; 4] = Default::default();
    for (shown, member) in characters.iter_mut().zip(state.party.members) {
        let record = member.and_then(|slot| state.party.characters.get(usize::from(slot)));
        *shown = record.map(|record| {
            BaseSavePreviewCharacter::new(
                i32::from(record.info.serial_no),
                i32::from(record.level),
                record.name.clone(),
            )
        });
    }
    BaseSavePreview {
        has_data: true,
        gil: i64::from(state.party.gil),
        play_duration: state.play_time_seconds as u64,
        win_type: state.carried.setting.win_type,
        location: place.to_owned(),
        timestamp,
        characters,
    }
}

/// Writes `state` into the file at `path`, creating the file when it does not exist. `place` is
/// the map name for the preview. Never touches another file.
pub fn write_slot(
    path: &Path,
    location: &LegacySaveLocation,
    state: &SaveState,
    place: &str,
) -> Result<(), String> {
    let payload = state
        .encode()
        .map_err(|error| format!("cannot encode the save: {}", error.context))?;
    let mut bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tantalus_legacy::saves::blank_container(Ff9SaveTree::PAYLOAD_BYTES)
        }
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    let preview = preview(state, place, now);
    tantalus_legacy::saves::write_save(&mut bytes, location, payload, Some(&preview), now)
        .map_err(|error| format!("cannot store the save: {}", error.context))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    }
    let temp = path.with_extension("dat.tmp");
    std::fs::write(&temp, &bytes)
        .map_err(|error| format!("cannot write {}: {error}", temp.display()))?;
    std::fs::rename(&temp, path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_legacy::battle::rewards::PaEntry;

    fn tables() -> MenuTables {
        let mut tables = MenuTables::default();
        // Zidane learns ability 20 (30 AP) and support ability 193 (20 AP); Freya has 25.
        tables.abilities.learnable.insert(
            0,
            vec![
                PaEntry { id: 20, max_ap: 30 },
                PaEntry {
                    id: 193,
                    max_ap: 20,
                },
            ],
        );
        tables
            .abilities
            .learnable
            .insert(4, vec![PaEntry { id: 25, max_ap: 40 }]);
        tables.exp = (0..99).map(|level| level * level * 10).collect();
        for (id, category) in [(5, 5), (8, 21)] {
            tables.abilities.defaults.insert(
                id,
                crate::ability_menu::CharacterDefaults {
                    category,
                    ..Default::default()
                },
            );
        }
        tables
    }

    fn played_world() -> FieldWorld {
        let mut world = FieldWorld::new();
        world.party = vec![0, 4, 7, 3];
        world.gil = 12_345;
        world.items.insert(236, 5);
        world.items.insert(7, 99);
        world.items.insert(31, 2);
        world.character_exp.insert(0, 9_000);
        world.character_exp.insert(4, 1_200);
        world.character_stats.insert(
            0,
            CharacterStats {
                current_hp: 400,
                max_hp: 650,
                current_mp: 30,
                max_mp: 88,
            },
        );
        world.equip(0, 0, Some(12));
        world.equip(0, 4, Some(200));
        world.equip(4, 1, Some(130));
        world.apply_ap(0, 20, 30, 45);
        world.apply_ap(0, 193, 20, 7);
        world.support_enabled.insert(0, vec![193]);
        world.trance.insert(0, 128);
        world.back_row.insert(4);
        world.cards = vec![3, 60];
        world.card_records = vec![
            tantalus_game::cards::CardRecord {
                id: 3,
                atk: 4,
                kind: 1,
                pdef: 2,
                mdef: 5,
                cpoint: 0,
                arrow: 0b1010_0101,
            },
            tantalus_game::cards::CardRecord {
                id: 60,
                atk: 9,
                kind: 3,
                pdef: 1,
                mdef: 1,
                cpoint: 2,
                arrow: 0xff,
            },
        ];
        world.card_stats = tantalus_game::cards::CardCounts {
            win: 4,
            lose: 2,
            draw: 1,
        };
        world
            .global_memory
            .set(0, tantalus_legacy::VariableType::UInt16, 4160);
        world
            .global_memory
            .set(2, tantalus_legacy::VariableType::UInt16, 17);
        world
            .global_memory
            .set(187, tantalus_legacy::VariableType::Byte, 0x5a);
        world.settings.fld_msg = 4;
        world.settings.atb = 1;
        world
    }

    fn temp_file(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tantalus-steam-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("SavedData_ww.dat")
    }

    #[test]
    fn slot_text_parses_the_three_forms() {
        let auto = parse_location("auto").unwrap();
        assert!(auto.is_autosave);
        let flat = parse_location("17").unwrap();
        assert_eq!((flat.slot_id, flat.save_id), (1, 2));
        let pair = parse_location("3:14").unwrap();
        assert_eq!((pair.slot_id, pair.save_id), (3, 14));
        assert!(parse_location("150").is_err());
        assert!(parse_location("10:0").is_err());
        assert!(parse_location("x").is_err());
    }

    #[test]
    fn old_slots_and_character_ids_agree() {
        // Memoria `GetPlayerFromOldSave`: slots 5 to 7 hold the guest while category bit 16 is set.
        for (slot, plain, guest) in [(5, 5, 8), (6, 6, 9), (7, 7, 10)] {
            assert_eq!(slot_character(slot, 6), Some(plain));
            assert_eq!(slot_character(slot, 21), Some(guest));
        }
        assert_eq!(slot_character(8, 22), Some(11));
        assert_eq!(slot_character(9, 0), None);
        assert_eq!(slot_character(4, 6), Some(4));
    }

    #[test]
    fn a_played_world_survives_save_file_and_load() {
        let tables = tables();
        let world = played_world();
        let state = capture(&world, 810, 7_200, &tables, &SaveState::empty()).unwrap();
        let path = temp_file("round-trip");
        let location = parse_location("17").unwrap();
        write_slot(&path, &location, &state, "Test Field").unwrap();
        // A second save in the same file keeps the first.
        let other = parse_location("2:5").unwrap();
        write_slot(&path, &other, &state, "Other").unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 2_937_152);
        assert_eq!(list_saves(&bytes).unwrap().len(), 2);
        let latest = latest_location(&bytes).unwrap();
        assert_eq!((latest.slot_id, latest.save_id), (2, 5));
        let loaded = read_slot(&path, &location).unwrap();
        assert_eq!(loaded.state, state);
        assert_eq!(loaded.state.field.fld_map_no, 810);
        assert_eq!(
            read_slot(&path, &parse_location("0").unwrap()).unwrap_err(),
            EMPTY_SLOT
        );

        let mut restored = FieldWorld::new();
        let resume = apply(&loaded.state, &mut restored, &tables).unwrap();
        assert_eq!(
            resume,
            Resume {
                map: 810,
                world: None
            }
        );
        assert_eq!(restored.party, world.party);
        assert_eq!(restored.gil, world.gil);
        assert_eq!(restored.items, world.items);
        assert_eq!(restored.character_exp, world.character_exp);
        assert_eq!(restored.character_stats[&0], world.character_stats[&0]);
        assert_eq!(restored.equipment[&0], world.equipment[&0]);
        assert_eq!(restored.equipment[&4], world.equipment[&4]);
        assert_eq!(restored.ability_pa[&0], world.ability_pa[&0]);
        assert_eq!(restored.learned_abilities[&0], world.learned_abilities[&0]);
        assert_eq!(restored.support_enabled[&0], vec![193]);
        assert_eq!(restored.trance[&0], 128);
        assert!(restored.back_row.contains(&4) && !restored.back_row.contains(&0));
        assert_eq!(restored.cards, world.cards);
        assert_eq!(restored.card_records, world.card_records);
        assert_eq!(restored.card_stats, world.card_stats);
        assert_eq!(restored.global_memory, world.global_memory);
        assert_eq!(restored.settings.fld_msg, 4);
        // 7,200 ticks at 60 a second.
        assert_eq!(loaded.state.play_time_seconds, 120.0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_guest_holds_its_slot_only_while_the_permanent_member_is_away() {
        let tables = tables();
        let mut world = FieldWorld::new();
        world.party = vec![0, 8];
        let state = capture(&world, 100, 0, &tables, &SaveState::empty()).unwrap();
        assert_eq!(state.party.characters[5].name, "Cinna");
        assert_eq!(state.party.characters[5].category & 16, 16);
        assert_eq!(state.party.members, [Some(0), Some(5), None, None]);
        let mut restored = FieldWorld::new();
        apply(&state, &mut restored, &tables).unwrap();
        assert_eq!(restored.party, vec![0, 8]);
        // With Quina in the party instead, slot 5 is Quina.
        world.party = vec![0, 5];
        let state = capture(&world, 100, 0, &tables, &state).unwrap();
        assert_eq!(state.party.characters[5].name, "Quina");
        assert_eq!(state.party.characters[5].category & 16, 0);
    }

    #[test]
    #[ignore = "needs a real save file in TANTALUS_REAL_SAVE"]
    fn a_real_save_loads_and_saves_back_the_same_game() {
        // Local only: `TANTALUS_REAL_SAVE` is a copy of a `SavedData_ww.dat`.
        let path = std::env::var_os("TANTALUS_REAL_SAVE").expect("TANTALUS_REAL_SAVE");
        let bytes = std::fs::read(path).unwrap();
        let tables = tables();
        let mut checked = 0;
        for (location, _) in list_saves(&bytes).unwrap() {
            let save = read_slot_bytes(&bytes, &location).unwrap();
            let mut world = FieldWorld::new();
            let resume = apply(&save.state, &mut world, &tables).unwrap();
            assert_eq!(resume.map, i32::from(save.state.field.fld_map_no));
            let back = capture(&world, resume.map, 0, &tables, &save.state).unwrap();
            let (a, b) = (&save.state.party, &back.party);
            assert_eq!(a.gil, b.gil);
            assert_eq!(a.members, b.members, "{location:?}");
            let stacks = |items: &[Option<ItemStack>]| {
                let mut list: Vec<(u8, u8)> = items
                    .iter()
                    .flatten()
                    .filter(|stack| stack.count > 0)
                    .map(|stack| (stack.id, stack.count))
                    .collect();
                list.sort_unstable();
                list
            };
            assert_eq!(stacks(&a.items), stacks(&b.items), "{location:?}");
            for (slot, (before, after)) in a.characters.iter().zip(&b.characters).enumerate() {
                assert_eq!(before.exp, after.exp, "{location:?} slot {slot}");
                assert_eq!(before.equip, after.equip, "{location:?} slot {slot}");
                assert_eq!(before.cur, after.cur, "{location:?} slot {slot}");
                assert_eq!(before.max, after.max, "{location:?} slot {slot}");
                assert_eq!(before.trance, after.trance, "{location:?} slot {slot}");
                assert_eq!(before.info.row, after.info.row, "{location:?} slot {slot}");
            }
            assert_eq!(save.state.variables, back.variables);
            let owned = |cards: &[Option<CardSlot>]| cards.iter().flatten().count();
            assert_eq!(owned(&save.state.cards.cards), owned(&back.cards.cards));
            checked += 1;
        }
        assert!(checked > 0, "the file holds no save");
    }
}
