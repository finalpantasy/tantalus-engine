//! Field battles for the headless chain: an `ENCOUNT` request resolves to its
//! battle scene (`audit/battle-scene-ids.tsv`, then `p0data2.bin`) and enemy
//! AI script (`EVT_BATTLE_<name>` in the event bundle), and runs on the legacy
//! battle machine through [`EncounterCoordinator`] with the party attacking.
//!
//! Stand-ins, each a gap to close rather than a claim about the game: party
//! units come from base stats and the field HP/MP at level 1; equipment is
//! each character's default set (`CharacterParameters.csv` ->
//! `DefaultEquipment.csv`), since the field side does not track equipment
//! yet; its stat bonuses (`Stats.csv`) are applied. Victories pay gil, items
//! and EXP into the field world; AP and cards are reported only. Every party member uses
//! Attack on the first living enemy, the status table is empty, and the
//! first pattern is used when the request names no group.

use std::collections::BTreeMap;
use std::path::Path;

use tantalus_assets::pc::battle_scene::{BattleBundleData, BattleScene};
use tantalus_contracts::{ContentId, EntityId, Tick};
use tantalus_game::FieldWorld;
use tantalus_game::battle::legacy::{LegacyBattleAdapter, unit_mappings};
use tantalus_game::battle::{
    BattleCommand, BattleOutcome, CombatEvent, CombatInput, WorldGameServices,
};
use tantalus_game::field::world::{CharacterStats, EncounterRequest};
use tantalus_game::new_game::BaseStats;
use tantalus_legacy::EventBinary;
use tantalus_legacy::battle::host::{apply_unit_views, battle_views};
use tantalus_legacy::battle::machine::{
    ABILITY_ATTACK, Battle, BattleConfig, BattleHooks, EnemyAction, Phase,
};
use tantalus_legacy::battle::queue::{CommandSpec, cmd};
use tantalus_legacy::battle::rng::SplitMix64Random;
use tantalus_legacy::battle::script_host::{
    AiEvent, BattleScriptDriver, BattleScriptState, EnemyAiHooks, SceneOverlay, ScriptTable,
};
use tantalus_legacy::battle::status::DEATH;
use tantalus_legacy::battle::targeting::target_type;
use tantalus_legacy::battle::{BattleUnit, Elements, StatusDataRow, StatusTable, WeaponStats};

use tantalus_legacy::battle::rewards::{
    BattleBonus, BattleReward, LevelBonus, PaEntry, PartyMember, PartySnapshot,
};

use crate::battle_scene_rewards::{pattern_machine_units, pattern_rewards};

/// Ticks a battle may run before the chain gives up on it.
pub const BATTLE_TICK_LIMIT: u64 = 20_000;

/// One line's AP: character, AP earned, and `(ability, max AP)` for each
/// equipped learnable ability.
pub type ApLine = (i32, u16, Vec<(i32, i32)>);

/// One fought battle, as the chain reports it.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct FightReport {
    pub battle: i32,
    /// Items the party used, `(item id, count)`. [`pay_rewards`] takes them
    /// out of the field inventory whatever the outcome.
    pub items_used: Vec<(i32, u32)>,
    pub scene: Option<String>,
    pub pattern: usize,
    pub outcome: Option<String>,
    pub ticks: u64,
    pub enemies: usize,
    pub enemies_defeated: usize,
    pub party_hp: Vec<(i32, u32, u32)>,
    /// Party MP at the end: character id, current, max.
    pub party_mp: Vec<(i32, u32, u32)>,
    pub enemy_hp: Vec<(u32, u32)>,
    /// Machine phase changes: tick, `phase/menu-off sequence`, enemy HP.
    pub phase_log: Vec<(u64, String, Vec<u32>)>,
    /// `btl_result` once the machine is over.
    pub result_code: Option<u8>,
    /// The first battle script error, if any.
    pub ai_error: Option<String>,
    /// Distinct script events worth a look: unmodeled reads and spawns,
    /// battle codes.
    pub ai_notes: Vec<String>,
    /// Machine phase and menu-off sequence when the fight stopped.
    pub phase: Option<String>,
    pub ai_script: bool,
    pub error: Option<String>,
    /// Victory pay (`BattleResultUI`): gil, EXP per character, AP per
    /// participating line, item stacks `(id, count)`, card.
    pub reward_gil: u32,
    pub reward_exp: Vec<(i32, u32)>,
    pub reward_ap: u16,
    pub reward_items: Vec<(u32, u32)>,
    pub reward_card: Option<u32>,
    /// AP per line that earns it: character, AP, and each equipped ability
    /// the character can learn with its AP goal `(ability, max AP)`, once
    /// per equipped item that carries it.
    pub reward_ap_lines: Vec<ApLine>,
    /// Units, positions and per-tick HP/ATB for a client to draw; runtime
    /// only.
    #[serde(skip)]
    pub stage: Option<crate::battle_stage::BattleStage>,
}

/// One party member going into battle.
#[derive(Debug, Clone, Copy)]
pub struct PartyFighter {
    pub character: i32,
    pub hp: u32,
    pub max_hp: u32,
    pub mp: u32,
    pub max_mp: u32,
    pub base: BaseStats,
    /// Event globals the battle setup reads.
    pub globals: PartyGlobals,
}

/// The event globals `btl_init` reads when it sets up a party member.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PartyGlobals {
    /// Scenario counter (global bytes 0-1), for the serial number formulas.
    pub scenario: i32,
    /// `battle.TRANCE_GAUGE_FLAG` (`gEventGlobal[16]`).
    pub trance_gauge: u8,
    /// `battle.GARNET_DEPRESS_FLAG` (`gEventGlobal[17]`).
    pub garnet_depress: u8,
}

impl PartyGlobals {
    /// From the field's global event memory.
    pub fn from_globals(bytes: &[u8]) -> Self {
        let byte = |index: usize| bytes.get(index).copied().unwrap_or(0);
        Self {
            scenario: i32::from(byte(0)) | (i32::from(byte(1) as i8) << 8),
            trance_gauge: byte(16),
            garnet_depress: byte(17),
        }
    }
}

pub struct FieldBattles {
    data: BattleBundleData,
    scripts: BTreeMap<String, Vec<u8>>,
    scene_ids: BTreeMap<u16, String>,
    equipment: Equipment,
}

/// Item ids below this are weapons; armor rows are `item - ARMOR_BASE`
/// (`ff9armor`, `Armors.csv` row 0 is item 88).
const ARMOR_BASE: i32 = 88;

/// Default equipment from the Memoria data tables.
#[derive(Debug, Clone, Default)]
pub struct Equipment {
    /// Character id -> `DefaultEquipmentSet`.
    sets: BTreeMap<i32, i32>,
    /// Set id -> weapon, head, wrist, armor, accessory item ids (-1 none).
    items: BTreeMap<i32, [i32; 5]>,
    /// Weapon item id -> stats.
    weapons: BTreeMap<i32, WeaponStats>,
    /// Armor row -> physical defence, physical evade, magic defence, magic
    /// evade.
    armors: BTreeMap<i32, [i32; 4]>,
    /// Item id -> `BonusId` (`Items.csv`).
    bonus_ids: BTreeMap<i32, i32>,
    /// Item id -> `GraphicsId` (`ff9item.shape`).
    shapes: BTreeMap<i32, u8>,
    /// `BonusId` -> speed, strength, magic, spirit (`Stats.csv`).
    bonuses: BTreeMap<i32, [i32; 4]>,
    /// Item id -> ability ids (`Items.csv` `AbilityIds`: `AA:X` is X,
    /// `SA:X` is 192 + X, `ff9abil.cs:24-25`).
    item_abilities: BTreeMap<i32, Vec<i32>>,
    /// Character id -> preset id (`CharacterParameters.csv`
    /// `DefaultCommandSet`, which equals the `CharacterPresetId`).
    presets: BTreeMap<i32, i32>,
    /// Preset id -> learnable abilities (`Abilities/<Preset>.csv`).
    learnable: BTreeMap<i32, Vec<PaEntry>>,
    /// `FF9Battle.add_status` (`Battle/StatusSets.csv`): set id -> status
    /// mask. Empty when the file is missing; every set is then no status.
    status_sets: BTreeMap<i32, u64>,
    /// `FF9Battle.aa_data` (`Battle/Actions.csv`): ability id -> row.
    actions: BTreeMap<i32, ActionRow>,
    /// Item id -> its `ITEM_DATA` (`Items.csv` `EffectId` into
    /// `ItemEffects.csv`), for usable items only.
    item_effects: BTreeMap<i32, ItemRow>,
}

/// The `ITEM_DATA` fields an Item command reads (`ItemEffects.csv`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ItemRow {
    target: u8,
    default_ally: bool,
    for_dead: bool,
    script_id: i32,
    power: i32,
    rate: i32,
    elements: u8,
    status: u64,
}

/// The `AA_DATA` fields `CMD_DATA.SetAAData` copies (`Actions.csv`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ActionRow {
    target: u8,
    default_ally: bool,
    for_dead: bool,
    script_id: i32,
    power: i32,
    elements: u8,
    rate: i32,
    category: u8,
    add_status: i32,
    mp: i32,
    ability_type: u8,
}

/// `CharacterPresetId` names, which are the ability file names.
const PRESET_FILES: [&str; 16] = [
    "Zidane", "Vivi", "Garnet", "Steiner", "Freya", "Quina", "Eiko", "Amarant", "Cinna1", "Cinna2",
    "Marcus1", "Marcus2", "Blank1", "Blank2", "Beatrix1", "Beatrix2",
];

/// Stand-in command lengths. The source holds a command in
/// `CMD_MODE_LOOP` until its SFX and sequence end, and the caster's ATB stays
/// still meanwhile (`sel_mode != 0`, `HonoluluBattleMain.cs:445`). The SFX
/// data is not ported, so every command takes the same 75 ticks (about 2.5 s
/// at 30 ticks a second) and applies its effect at tick 40. With 0 a fast
/// enemy acted about three times per party turn.
const COMMAND_TIMING: tantalus_legacy::battle::machine::Timing =
    tantalus_legacy::battle::machine::Timing {
        enemy_death_anim_ticks: 0,
        player_death_ticks: 0,
        effect_after_loop_ticks: 40,
        loop_total_ticks: 75,
    };

/// `BattleCommandId.BlackMagic`, `InsideTrick` (Dyne) and the ability ids
/// the stand-in policy uses (`Actions.csv`).
const BLACK_MAGIC: u32 = 22;
const INSIDE_TRICK: u32 = 26;
const ABILITY_FIRE: i32 = 25;
const ABILITY_FLEE: i32 = 101;
const ABILITY_FREE_ENERGY: i32 = 109;
const ITEM_POTION: i32 = 236;
const ITEM_HI_POTION: i32 = 237;
const ITEM_PHOENIX_DOWN: i32 = 240;
const ITEM_PHOENIX_PINION: i32 = 249;
const ITEM_REMEDY: i32 = 247;

/// `ManyAny(3)` -> `3`, a bare number as is.
fn paren_or_number(text: &str) -> String {
    match (text.find('('), text.find(')')) {
        (Some(open), Some(close)) if open < close => text[open + 1..close].trim().to_string(),
        _ => text.trim().to_string(),
    }
}

/// `Poison(16), Sleep(17)` -> bits 16 and 17.
fn status_mask(list: &str) -> u64 {
    list.split(',')
        .filter_map(|item| {
            let open = item.find('(')?;
            let close = item[open..].find(')')? + open;
            item[open + 1..close].trim().parse::<u32>().ok()
        })
        .filter(|&bit| bit < 64)
        .fold(0, |mask, bit| mask | 1u64 << bit)
}

/// The enemy side of `btl_util.GetCommandAction` (`btl_util.cs:422`):
/// every command an enemy issues reads `FF9Battle.enemy_attack[sub_no]`,
/// the scene's attack list, through `CMD_DATA.SetAAData`
/// (`CMD_DATA.cs:39-55`). Player commands keep the basic catalog.
struct SceneAttacks {
    attacks: Vec<tantalus_assets::pc::battle_scene::Attack>,
    status_sets: BTreeMap<i32, u64>,
    actions: BTreeMap<i32, ActionRow>,
    item_effects: BTreeMap<i32, ItemRow>,
}

impl SceneAttacks {
    /// A player ability command (`aa_data[sub_no]`, `btl_util.cs:427`).
    fn player_spec(&self, cmd_no: u32, sub_no: i32) -> Option<CommandSpec> {
        let row = self.actions.get(&sub_no)?;
        let mut command = tantalus_legacy::battle::BattleCommand::new(row.script_id);
        command.command_id = cmd_no;
        command.power = row.power;
        command.hit_rate = row.rate;
        command.element = Elements(row.elements);
        command.element_for_bonus = command.element;
        command.ability_status = self.status_sets.get(&row.add_status).copied().unwrap_or(0);
        command.ability_category = row.category;
        command.ability_type = row.ability_type;
        command.target_type = row.target;
        command.is_many_target =
            !(row.target <= target_type::MANY_ENEMY || row.target >= target_type::SELF);
        command.mp_cost = row.mp;
        Some(CommandSpec {
            command,
            default_ally: row.default_ally,
            for_dead: row.for_dead,
        })
    }
}

impl BattleHooks for SceneAttacks {
    fn spec(&mut self, cmd_no: u32, sub_no: i32, caster: &BattleUnit) -> Option<CommandSpec> {
        if caster.is_player && cmd_no != cmd::MAGIC_COUNTER {
            return match cmd_no {
                BLACK_MAGIC | INSIDE_TRICK => self.player_spec(cmd_no, sub_no),
                // The base the machine's `item_spec` fills in: the item's
                // cursor and default side.
                cmd::ITEM => self.item_effects.get(&sub_no).map(|item| {
                    let mut command = tantalus_legacy::battle::BattleCommand::new(item.script_id);
                    command.command_id = cmd_no;
                    command.target_type = item.target;
                    CommandSpec {
                        command,
                        default_ally: item.default_ally,
                        for_dead: item.for_dead,
                    }
                }),
                _ => tantalus_game::battle::legacy::basic_spec(cmd_no),
            };
        }
        let attack = self.attacks.get(usize::try_from(sub_no).ok()?)?;
        let mut command = tantalus_legacy::battle::BattleCommand::new(attack.script_id);
        command.command_id = cmd_no;
        command.power = attack.power;
        command.hit_rate = attack.rate;
        let element = match attack.elements {
            tantalus_assets::pc::battle_scene::ByteOrTokens::Byte(bits) => bits,
            _ => 0,
        };
        command.element = Elements(element);
        command.element_for_bonus = command.element;
        command.ability_status = self
            .status_sets
            .get(&attack.add_status_no)
            .copied()
            .unwrap_or(0);
        command.ability_category = attack.category;
        command.ability_type = attack.attack_type;
        command.target_type = attack.target;
        command.is_many_target =
            !(attack.target <= target_type::MANY_ENEMY || attack.target >= target_type::SELF);
        // `IsAttackShortRange` (`btl_util.cs:311-326`): an enemy caster has
        // no weapon, so every enemy attack counts as short range.
        command.is_short_range = true;
        command.mp_cost = attack.mp;
        Some(CommandSpec {
            command,
            default_ally: attack.default_ally,
            for_dead: attack.for_dead,
        })
    }

    fn enemy_atb(&mut self, _battle: &Battle, _unit: usize) -> Option<EnemyAction> {
        None
    }

    /// `ff9item.GetItemEffect` (`ITEM_DATA`).
    fn item(&mut self, item_id: i32) -> Option<tantalus_legacy::battle::command::ItemData> {
        let item = self.item_effects.get(&item_id)?;
        Some(tantalus_legacy::battle::command::ItemData {
            effect: tantalus_legacy::battle::command::ItemEffect {
                power: item.power,
                hit_rate: item.rate,
                status: item.status,
                elements: Elements(item.elements),
            },
            script_id: item.script_id,
            for_dead: item.for_dead,
        })
    }
}

/// `AA:X` -> X, `SA:X` -> 192 + X, a bare number as is; 0 is the void entry.
fn ability_id(text: &str) -> Option<i32> {
    let text = text.trim();
    if let Some(value) = text.strip_prefix("AA:") {
        value.trim().parse().ok()
    } else if let Some(value) = text.strip_prefix("SA:") {
        value.trim().parse::<i32>().ok().map(|id| 192 + id)
    } else {
        text.parse().ok()
    }
}

impl Equipment {
    /// Reads `CharacterParameters.csv` and `DefaultEquipment.csv` from
    /// `characters` and `Weapons.csv` / `Armors.csv` from `../Items`.
    pub fn load(characters: &Path) -> Result<Self, String> {
        let read = |path: std::path::PathBuf| {
            std::fs::read(&path)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .map_err(|error| format!("cannot read {}: {error}", path.display()))
        };
        let items_dir = characters.join("../Items");
        let mut equipment = Self::default();
        for row in csv_rows(&read(characters.join("CharacterParameters.csv"))?) {
            if let (Some(id), Some(set)) = (int(&row, 0), int(&row, 5)) {
                equipment.sets.insert(id, set);
            }
            if let (Some(id), Some(preset)) = (int(&row, 0), int(&row, 4)) {
                equipment.presets.insert(id, preset);
            }
        }
        // Id;AP per row; `0` rows are void and keep their place.
        for (preset, name) in PRESET_FILES.iter().enumerate() {
            let path = characters.join("Abilities").join(format!("{name}.csv"));
            let Ok(text) = read(path) else { continue };
            let rows = csv_rows(&text)
                .filter_map(|row| {
                    Some(PaEntry {
                        id: ability_id(row.first()?)?,
                        max_ap: int(&row, 1)?,
                    })
                })
                .collect();
            equipment.learnable.insert(preset as i32, rows);
        }
        for row in csv_rows(&read(characters.join("DefaultEquipment.csv"))?) {
            let Some(id) = int(&row, 1) else { continue };
            let mut slots = [-1; 5];
            for (slot, value) in slots.iter_mut().enumerate() {
                *value = int(&row, 2 + slot).unwrap_or(-1);
            }
            equipment.items.insert(id, slots);
        }
        // Comment;Id;Category;StatusIndex;ModelName;ScriptId;Power;Elements;Rate;...
        for row in csv_rows(&read(items_dir.join("Weapons.csv"))?) {
            let (Some(id), Some(category), Some(power)) =
                (int(&row, 1), int(&row, 2), int(&row, 6))
            else {
                continue;
            };
            equipment.weapons.insert(
                id,
                WeaponStats {
                    power,
                    rate: int(&row, 8).unwrap_or(0),
                    elements: Elements::NONE,
                    category: category as u8,
                    status: 0,
                },
            );
        }
        // Comment;Id;PhysicalDefence;PhysicalEvade;MagicalDefence;MagicalEvade
        for row in csv_rows(&read(items_dir.join("Armors.csv"))?) {
            let Some(id) = int(&row, 1) else { continue };
            let mut stats = [0; 4];
            for (index, value) in stats.iter_mut().enumerate() {
                *value = int(&row, 2 + index).unwrap_or(0);
            }
            equipment.armors.insert(id, stats);
        }
        // Id;WeaponId;ArmorId;EffectId;Price;SellingPrice;GraphicsId;ColorId;Quality;BonusId;...
        for row in csv_rows(&read(items_dir.join("Items.csv"))?) {
            if let (Some(id), Some(bonus)) = (int(&row, 0), int(&row, 9)) {
                equipment.bonus_ids.insert(id, bonus);
            }
            if let (Some(id), Some(shape)) = (int(&row, 0), int(&row, 6)) {
                equipment.shapes.insert(id, shape.clamp(0, 255) as u8);
            }
            if let (Some(id), Some(abilities)) = (int(&row, 0), row.get(10)) {
                let abilities: Vec<i32> = abilities.split(',').filter_map(ability_id).collect();
                if abilities.iter().any(|&ability| ability != 0) {
                    equipment.item_abilities.insert(id, abilities);
                }
            }
        }
        // Comment;Id;Speed;Strength;Magic;Spirit;...
        for row in csv_rows(&read(items_dir.join("Stats.csv"))?) {
            let Some(id) = int(&row, 1) else { continue };
            let mut stats = [0; 4];
            for (index, value) in stats.iter_mut().enumerate() {
                *value = int(&row, 2 + index).unwrap_or(0);
            }
            equipment.bonuses.insert(id, stats);
        }
        // Comment;Id;Statuses, each status `Name(id)` (`#! UnshiftStatuses`:
        // the id is the bit index).
        if let Ok(text) = read(characters.join("../Battle/StatusSets.csv")) {
            for row in csv_rows(&text) {
                let Some(id) = int(&row, 1) else { continue };
                let mask = row.get(2).map(|list| status_mask(list)).unwrap_or(0);
                equipment.status_sets.insert(id, mask);
            }
        }
        // Id;Targets;DefaultAlly;Display;AnimationId;ForDead;DefaultDead;
        // ScriptId;Power;Rate;Elements;Status, keyed through Items.csv's
        // EffectId (column 3).
        if let (Ok(items), Ok(effects)) = (
            read(items_dir.join("Items.csv")),
            read(items_dir.join("ItemEffects.csv")),
        ) {
            let mut rows = BTreeMap::new();
            for row in csv_rows(&effects) {
                let Some(id) = int(&row, 0) else { continue };
                let field = |index: usize| row.get(index).copied().unwrap_or("");
                let flag = |index: usize| matches!(field(index).trim(), "1" | "True" | "true");
                rows.insert(
                    id,
                    ItemRow {
                        target: paren_or_number(field(1)).parse().unwrap_or(0),
                        default_ally: flag(2),
                        for_dead: flag(5),
                        script_id: int(&row, 7).unwrap_or(0),
                        power: int(&row, 8).unwrap_or(0),
                        rate: int(&row, 9).unwrap_or(0),
                        elements: status_mask(field(10)) as u8,
                        status: status_mask(field(11)),
                    },
                );
            }
            for row in csv_rows(&items) {
                if let (Some(id), Some(effect)) = (int(&row, 0), int(&row, 3))
                    && let Some(item) = rows.get(&effect)
                {
                    equipment.item_effects.insert(id, *item);
                }
            }
        }
        // Comment;Id;Display;Targets;DefaultAlly;ForDead;DefaultDead;
        // DefaultCamera;AnimationId;AnimationId2;ScriptId;Power;Elements;
        // Rate;Category;AddStatus;MP;Type;...
        if let Ok(text) = read(characters.join("../Battle/Actions.csv")) {
            for row in csv_rows(&text) {
                let Some(id) = int(&row, 1) else { continue };
                let field = |index: usize| row.get(index).copied().unwrap_or("");
                let flag = |index: usize| matches!(field(index).trim(), "1" | "True" | "true");
                let byte = |index: usize| int(&row, index).unwrap_or(0).clamp(0, 255) as u8;
                equipment.actions.insert(
                    id,
                    ActionRow {
                        target: paren_or_number(field(3)).parse().unwrap_or(0),
                        default_ally: flag(4),
                        for_dead: flag(5),
                        script_id: int(&row, 10).unwrap_or(0),
                        power: int(&row, 11).unwrap_or(0),
                        elements: status_mask(field(12)) as u8,
                        rate: int(&row, 13).unwrap_or(0),
                        category: byte(14),
                        add_status: int(&row, 15).unwrap_or(0),
                        mp: int(&row, 16).unwrap_or(0),
                        ability_type: byte(17),
                    },
                );
            }
        }
        Ok(equipment)
    }

    /// The default weapon's `GraphicsId` (`WeaponShape`), 0 without one.
    fn weapon_shape(&self, character: i32) -> u8 {
        self.sets
            .get(&character)
            .and_then(|set| self.items.get(set))
            .and_then(|slots| self.shapes.get(&slots[0]))
            .copied()
            .unwrap_or(0)
    }

    /// The character's default weapon and summed armor defences.
    fn outfit(&self, character: i32) -> (Option<WeaponStats>, [i32; 4]) {
        let Some(slots) = self
            .sets
            .get(&character)
            .and_then(|set| self.items.get(set))
        else {
            return (None, [0; 4]);
        };
        let weapon = self.weapons.get(&slots[0]).copied();
        let mut defence = [0; 4];
        for &item in &slots[1..] {
            if let Some(stats) = self.armors.get(&(item - ARMOR_BASE)) {
                for (total, value) in defence.iter_mut().zip(stats) {
                    *total += value;
                }
            }
        }
        (weapon, defence)
    }

    /// `itemData.ability` of every default-equipped item, slot order, and
    /// the character's learnable table (`_FF9Abil_PaData[PresetId]`).
    fn abilities(&self, character: i32) -> (Vec<i32>, Vec<PaEntry>) {
        let equipped = self
            .sets
            .get(&character)
            .and_then(|set| self.items.get(set))
            .map(|slots| {
                slots
                    .iter()
                    .filter_map(|item| self.item_abilities.get(item))
                    .flatten()
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        let learnable = self
            .presets
            .get(&character)
            .and_then(|preset| self.learnable.get(preset))
            .cloned()
            .unwrap_or_default();
        (equipped, learnable)
    }

    /// The equipment half of `ff9play.FF9Play_Update`: speed, strength,
    /// magic and spirit bonuses summed over the five equipped items.
    fn stat_bonus(&self, character: i32) -> [i32; 4] {
        let mut total = [0; 4];
        let Some(slots) = self
            .sets
            .get(&character)
            .and_then(|set| self.items.get(set))
        else {
            return total;
        };
        for item in slots {
            if let Some(stats) = self
                .bonus_ids
                .get(item)
                .and_then(|bonus| self.bonuses.get(bonus))
            {
                for (sum, value) in total.iter_mut().zip(stats) {
                    *sum += value;
                }
            }
        }
        total
    }
}

/// Data rows of a Memoria `;` table: `#` lines are comments.
fn csv_rows(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .map(|line| line.trim_start_matches('\u{feff}'))
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| line.split(';').collect())
}

fn int(row: &[&str], index: usize) -> Option<i32> {
    row.get(index)?.trim().parse().ok()
}

impl FieldBattles {
    /// `streaming_assets` holds `p0data2.bin` (scenes); `event_bundle` holds
    /// the `EVT_BATTLE_*` AI scripts; `scene_ids` is the id table.
    pub fn load(
        streaming_assets: &Path,
        event_bundle: &Path,
        scene_ids: &Path,
        equipment: Equipment,
    ) -> Result<Self, String> {
        let data = BattleBundleData::load(&streaming_assets.join("p0data2.bin"))
            .map_err(|error| error.to_string())?;
        let scripts = crate::battle_ai_demo::load_battle_scripts(event_bundle)?;
        let table = std::fs::read_to_string(scene_ids)
            .map_err(|error| format!("cannot read {}: {error}", scene_ids.display()))?;
        Ok(Self {
            data,
            scripts,
            scene_ids: crate::battle_scene_rewards::parse_scene_id_table(&table),
            equipment,
        })
    }

    /// Test and tooling constructor from already-loaded parts (no disk).
    pub fn from_parts(
        data: BattleBundleData,
        scripts: BTreeMap<String, Vec<u8>>,
        scene_ids: BTreeMap<u16, String>,
        equipment: Equipment,
    ) -> Self {
        Self {
            data,
            scripts,
            scene_ids,
            equipment,
        }
    }

    /// Fights `request` with `party`. [`save_party`] and [`pay_rewards`] carry
    /// the result back to the field.
    pub fn fight(&self, request: &EncounterRequest, party: &[PartyFighter]) -> FightReport {
        self.fight_with_items(request, party, &BTreeMap::new())
    }

    /// [`Self::fight`] with the field inventory (`item id -> count`), which
    /// the Item command reads. The report lists what was used.
    pub fn fight_with_items(
        &self,
        request: &EncounterRequest,
        party: &[PartyFighter],
        items: &BTreeMap<i32, i32>,
    ) -> FightReport {
        let pattern = request.group.and_then(|group| usize::try_from(group).ok());
        let mut report = FightReport {
            battle: request.battle,
            scene: None,
            pattern: pattern.unwrap_or(0),
            outcome: None,
            ticks: 0,
            enemies: 0,
            enemies_defeated: 0,
            party_hp: Vec::new(),
            party_mp: Vec::new(),
            enemy_hp: Vec::new(),
            phase_log: Vec::new(),
            result_code: None,
            ai_error: None,
            ai_notes: Vec::new(),
            phase: None,
            ai_script: false,
            error: None,
            ..FightReport::default()
        };
        if let Err(error) = self.run(request, party, items, &mut report) {
            report.error = Some(error);
        }
        report
    }

    fn run(
        &self,
        request: &EncounterRequest,
        party: &[PartyFighter],
        items: &BTreeMap<i32, i32>,
        report: &mut FightReport,
    ) -> Result<(), String> {
        let id = u16::try_from(request.battle).map_err(|_| "battle id out of range")?;
        let key = self
            .scene_ids
            .get(&id)
            .ok_or_else(|| format!("battle {id} is not in the scene id table"))?;
        let name = crate::battle_scene_rewards::event_asset_name(key)
            .ok_or_else(|| format!("scene key `{key}` has no BSC_ prefix"))?;
        report.scene = Some(name.clone());
        let (_, scene) = self
            .data
            .scene(&name)
            .ok_or_else(|| format!("scene `{name}` is not in p0data2.bin"))?;
        let scene = scene.as_ref().map_err(|error| error.to_string())?;
        let pattern = report.pattern.min(scene.patterns.len().saturating_sub(1));
        report.pattern = pattern;
        let script = self.scripts.get(&name);
        report.ai_script = script.is_some();
        self.battle(
            scene,
            pattern,
            script.map(Vec::as_slice),
            party,
            items,
            report,
        )
    }

    fn battle(
        &self,
        scene: &BattleScene,
        pattern: usize,
        script: Option<&[u8]>,
        party: &[PartyFighter],
        items: &BTreeMap<i32, i32>,
        report: &mut FightReport,
    ) -> Result<(), String> {
        let mut units: Vec<BattleUnit> = party
            .iter()
            .enumerate()
            .map(|(slot, fighter)| party_unit(slot, fighter, &self.equipment))
            .collect();
        let mut pairs: Vec<(EntityId, u16)> = units
            .iter()
            .map(|unit| (EntityId::new(1000 + u64::from(unit.id)), unit.id))
            .collect();
        let enemies = pattern_machine_units(scene, pattern)
            .ok_or_else(|| format!("pattern {pattern} has no machine units"))?;
        report.enemies = enemies.len();
        for unit in enemies {
            pairs.push((EntityId::new(u64::from(unit.id)), unit.id));
            units.push(unit);
        }
        let entities: Vec<EntityId> = pairs.iter().map(|pair| pair.0).collect();
        let mut battle = Battle::new(
            units,
            BattleConfig {
                timing: COMMAND_TIMING,
                ..BattleConfig::default()
            },
            StatusTable::new(vec![StatusDataRow::default(); 32]),
        );
        battle.inventory = items
            .iter()
            .filter(|&(_, &count)| count > 0)
            .map(|(&item, &count)| (item, count as u32))
            .collect();
        // `SB2_FLG_SPECIAL` is `Info.SpecialStart`: the script opens the
        // battle in the event phase.
        if scene.header.flags & 0x1 != 0 {
            battle.begin_special_start();
        }
        let table = match script {
            Some(bytes) => {
                ScriptTable::from_binary(&EventBinary::parse(bytes).map_err(|e| e.to_string())?)
            }
            None => ScriptTable { entries: vec![] },
        };
        let (data, views) = battle_views(&battle);
        let party_ids: Vec<u8> = party
            .iter()
            .map(|fighter| fighter.character as u8)
            .collect();
        let driver = BattleScriptDriver::new(
            table,
            BattleScriptState::new(data, views),
            Box::new(SplitMix64Random::new(u64::from(report.battle as u32))),
            &party_ids,
        );
        let (unit_map, target_bits) = unit_mappings(&pairs);
        let attack = ContentId::parse("tantalus:action/attack").map_err(|e| e.to_string())?;
        let fire = ContentId::parse("tantalus:action/fire").map_err(|e| e.to_string())?;
        let free_energy =
            ContentId::parse("tantalus:action/free-energy").map_err(|e| e.to_string())?;
        let potion = ContentId::parse("tantalus:action/potion").map_err(|e| e.to_string())?;
        let hi_potion = ContentId::parse("tantalus:action/hi-potion").map_err(|e| e.to_string())?;
        let pinion =
            ContentId::parse("tantalus:action/phoenix-pinion").map_err(|e| e.to_string())?;
        let remedy = ContentId::parse("tantalus:action/remedy").map_err(|e| e.to_string())?;
        let phoenix =
            ContentId::parse("tantalus:action/phoenix-down").map_err(|e| e.to_string())?;
        // Abilities each member's default gear gives (`AA:X` ids under 192).
        let gear: Vec<Vec<i32>> = party
            .iter()
            .map(|fighter| self.equipment.abilities(fighter.character).0)
            .collect();
        let adapter = LegacyBattleAdapter::new(
            battle,
            BTreeMap::from([
                (attack.clone(), (cmd::ATTACK, ABILITY_ATTACK)),
                (fire.clone(), (BLACK_MAGIC, ABILITY_FIRE)),
                (free_energy.clone(), (INSIDE_TRICK, ABILITY_FREE_ENERGY)),
                (potion.clone(), (cmd::ITEM, ITEM_POTION)),
                (hi_potion.clone(), (cmd::ITEM, ITEM_HI_POTION)),
                (pinion.clone(), (cmd::ITEM, ITEM_PHOENIX_PINION)),
                (remedy.clone(), (cmd::ITEM, ITEM_REMEDY)),
                (phoenix.clone(), (cmd::ITEM, ITEM_PHOENIX_DOWN)),
            ]),
            unit_map,
            target_bits,
        );
        let hooks = EnemyAiHooks::new(
            SceneAttacks {
                attacks: scene.attacks.clone(),
                status_sets: self.equipment.status_sets.clone(),
                actions: self.equipment.actions.clone(),
                item_effects: self.equipment.item_effects.clone(),
            },
            driver,
            SceneOverlay {
                battle_map_no: report.battle,
                pattern_id: pattern as i32,
                // `BTL_START_NORMAL_ATTACK`; preemptive and back attacks
                // are not rolled.
                start_type: 2,
                // Every `ff9btl.LOAD_*` bit, after the frames the load
                // loop takes (see `SceneOverlay::load_frames`).
                load_status: 0x7F,
                load_frames: 3,
                ..SceneOverlay::default()
            },
        );
        let mut adapter = adapter.with_hooks(hooks);
        let mut stage = crate::battle_stage::BattleStage::new(
            report.battle,
            // `EVT_BATTLE_<scene>` to the bare scene name.
            report
                .scene
                .as_deref()
                .map(|name| name.strip_prefix("EVT_BATTLE_").unwrap_or(name))
                .unwrap_or_default(),
            scene,
            pattern,
            &adapter.battle().units[..party.len().min(adapter.battle().units.len())],
        );
        let mut world = FieldWorld::new();
        let mut defeated: Vec<EntityId> = Vec::new();
        // `btl_sys.SetBonus` on each enemy death; `Info.AfterEvent` scenes
        // pay nothing (`battle.cs:72`).
        let mut rows = pattern_rewards(scene, pattern).unwrap_or_default();
        let pattern_ap = scene.patterns.get(pattern).map_or(0, |row| row.ap);
        let member_flag = (1u8 << party.len().min(4)) - 1;
        let mut bonus = BattleBonus::new(pattern_ap, member_flag, scene.header.flags & 2048 != 0);
        let mut draws = SplitMix64Random::new(u64::from(report.battle as u32) ^ 0xD1AA);
        let mut events_seen = 0usize;
        let mut sent = 0u32;
        for tick in 0..BATTLE_TICK_LIMIT {
            report.ticks = tick + 1;
            // The first ready party member attacks the first enemy still
            // standing, read from the machine (units are party then enemies).
            let mut input = CombatInput::new(Tick::new(tick));
            let machine = adapter.battle();
            let target = (party.len()..machine.units.len()).find(|&i| {
                machine.sched[i].in_list
                    && machine.units[i].cur_hp > 0
                    && !machine.units[i].is_under_any_status(DEATH)
            });
            let actor = machine
                .ready_players
                .iter()
                .copied()
                .find(|&i| i < party.len());
            if let (Some(actor), Some(target)) = (actor, target) {
                sent += 1;
                // Stand-in choices a player makes: Vivi casts Fire while he
                // has the MP, and Zidane in Trance uses Free Energy, open
                // while its Skill twin Flee is equipped (`BattleHUD.cs:1368`,
                // `AbilityTranceList`). Everything else attacks.
                let unit = &machine.units[actor];
                let has = |id: i32| gear[actor].contains(&id);
                let count = |item: i32| machine.inventory.get(&item).copied().unwrap_or(0);
                // A fallen member gets a Phoenix Down or Pinion (Vivi first, the
                // damage dealer, who never stops to heal), then the member under 60% HP with the least
                // left gets a Potion, or a Hi-Potion once those run out.
                // An ally already named by a queued or running Item command
                // is skipped, so two members do not spend turns on one heal.
                let healing = |i: usize| {
                    let id = machine.units[i].id;
                    machine.cmds.queue.iter().chain(&machine.running).any(|&c| {
                        let slot = &machine.cmds.slots[c];
                        slot.cmd_no == cmd::ITEM && slot.tar_id & id != 0
                    })
                };
                let downed = (0..party.len())
                    .filter(|&i| {
                        machine.sched[i].in_list && machine.units[i].cur_hp == 0 && !healing(i)
                    })
                    .min_by_key(|&i| party[i].character != 1);
                let hurt = (0..party.len())
                    .filter(|&i| {
                        let u = &machine.units[i];
                        machine.sched[i].in_list
                            && u.cur_hp > 0
                            && u.cur_hp * 5 < u.max_hp * 3
                            && !healing(i)
                    })
                    .min_by_key(|&i| {
                        machine.units[i].cur_hp * 1000 / machine.units[i].max_hp.max(1)
                    });
                // Vivi keeps casting while he has the MP; the others heal.
                let caster = party[actor].character == 1 && has(ABILITY_FIRE) && unit.cur_mp >= 6;
                let heal = match (downed, hurt) {
                    _ if caster => None,
                    (Some(ally), _) if count(ITEM_PHOENIX_DOWN) > 0 => {
                        Some((phoenix.clone(), ally))
                    }
                    (Some(ally), _) if count(ITEM_PHOENIX_PINION) > 0 => {
                        Some((pinion.clone(), ally))
                    }
                    (_, Some(ally)) if count(ITEM_POTION) > 0 => Some((potion.clone(), ally)),
                    (_, Some(ally)) if count(ITEM_HI_POTION) > 0 => Some((hi_potion.clone(), ally)),
                    // A blinded fighter's attacks mostly miss: Remedy.
                    _ if count(ITEM_REMEDY) > 0 => (0..party.len())
                        .find(|&i| {
                            machine.sched[i].in_list
                                && machine.units[i].cur_hp > 0
                                && party[i].character != 1
                                && machine.units[i]
                                    .is_under_any_status(tantalus_legacy::battle::status::BLIND)
                                && !healing(i)
                        })
                        .map(|ally| (remedy.clone(), ally)),
                    _ => None,
                };
                let (action, target) = if let Some(heal) = heal {
                    heal
                } else {
                    (
                        match party[actor].character {
                            1 if has(ABILITY_FIRE) && unit.cur_mp >= 6 => fire.clone(),
                            0 if has(ABILITY_FLEE)
                                && unit.is_under_any_status(
                                    tantalus_legacy::battle::status::TRANCE,
                                ) =>
                            {
                                free_energy.clone()
                            }
                            _ => attack.clone(),
                        },
                        target,
                    )
                };
                input.command = Some(BattleCommand {
                    actor: entities[actor],
                    action,
                    targets: vec![entities[target]],
                });
            }
            let mut services = WorldGameServices::new(&mut world);
            let advanced = adapter.advance(&input, &mut services);
            drop(services);
            // Script writes to units (`RunBattleCode` member sets) go back
            // into the machine, as in `battle_run`.
            let dirty = adapter.hooks_mut().take_dirty_views();
            apply_unit_views(adapter.battle_mut(), &dirty);
            let machine = adapter.battle();
            let phase = format!("{:?}/{:?}", machine.phase, machine.menu_off_seq);
            let (over, result) = (machine.over, machine.result);
            {
                let (max_atb, present): (Vec<i16>, Vec<bool>) = machine
                    .sched
                    .iter()
                    .map(|sched| (sched.max_atb, sched.in_list))
                    .unzip();
                stage.record(tick, &machine.units, &max_atb, &present, &phase);
            }
            if report.phase_log.last().map(|entry| &entry.1) != Some(&phase) {
                let hp = machine.units[party.len()..]
                    .iter()
                    .map(|unit| unit.cur_hp)
                    .collect();
                report.phase_log.push((tick, phase, hp));
                if over && let Some(result) = result {
                    report.result_code = Some(result);
                }
            }
            if let Err(error) = advanced {
                // A result the adapter has no outcome for (5 is a scripted
                // interruption, `BTL_RESULT_INTERRUPTION`).
                match (over, result) {
                    (true, Some(result)) => {
                        report.result_code = Some(result);
                        report.outcome = Some(format!("result {result}"));
                        break;
                    }
                    _ => return Err(format!("{error:?}")),
                }
            }
            for event in adapter.take_events() {
                if let CombatEvent::UnitDefeated { unit, .. } = event
                    && let Some(enemy) = entities[party.len()..].iter().position(|e| *e == unit)
                    && !defeated.contains(&unit)
                {
                    defeated.push(unit);
                    if enemy < rows.len() {
                        bonus.set_bonus(&mut rows, enemy, &mut draws);
                    }
                }
            }
            // Script-paid bonuses (member 71) for units that never die.
            let events = &adapter.hooks().driver.state.events;
            for event in &events[events_seen.min(events.len())..] {
                if let AiEvent::SetBonus { unit } = event
                    && let Some(enemy) = entities[party.len()..].iter().position(|e| e == unit)
                    && enemy < rows.len()
                {
                    bonus.set_bonus(&mut rows, enemy, &mut draws);
                }
            }
            events_seen = events.len();
            report.items_used = items
                .iter()
                .filter_map(|(&item, &count)| {
                    let left = adapter.battle().inventory.get(&item).copied().unwrap_or(0);
                    let used = (count.max(0) as u32).saturating_sub(left);
                    (used > 0).then_some((item, used))
                })
                .collect();
            if let Some(outcome) = adapter.outcome() {
                report.outcome = Some(outcome_name(outcome).to_owned());
                break;
            }
            // The machine does not reach `over` on a defeat (the defeat song
            // fade is not ported), so the defeat phase ends the fight here.
            if adapter.battle().phase == Phase::Defeat {
                report.outcome = Some("defeat".to_owned());
                break;
            }
        }
        stage.outcome = report.outcome.clone();
        report.stage = Some(stage);
        report.enemies_defeated = defeated.len();
        let machine = adapter.battle();
        report.party_hp = party
            .iter()
            .zip(&machine.units)
            .map(|(fighter, unit)| (fighter.character, unit.cur_hp, unit.max_hp))
            .collect();
        report.party_mp = party
            .iter()
            .zip(&machine.units)
            .map(|(fighter, unit)| (fighter.character, unit.cur_mp, unit.max_mp))
            .collect();
        report.enemy_hp = machine.units[party.len()..]
            .iter()
            .map(|unit| (unit.cur_hp, unit.max_hp))
            .collect();
        report.phase = Some(format!("{:?}/{:?}", machine.phase, machine.menu_off_seq));
        if report.outcome.as_deref() == Some("victory") {
            let mut members: [Option<PartyMember>; 4] = Default::default();
            for (slot, (fighter, unit)) in party.iter().zip(&machine.units).take(4).enumerate() {
                members[slot] = Some(reward_member(
                    fighter,
                    unit.cur_hp,
                    unit.cur_mp,
                    &self.equipment,
                ));
            }
            let snapshot = PartySnapshot {
                gil: 0,
                members,
                items: Vec::new(),
                cards: Vec::new(),
            };
            let reward = BattleReward::from_bonus(&bonus, &snapshot);
            report.reward_gil = reward.gil;
            report.reward_exp = party
                .iter()
                .zip(reward.shares)
                .filter_map(|(fighter, share)| share.exp.map(|exp| (fighter.character, exp)))
                .collect();
            // `AfterEvent` scenes show no result screen and pay no AP.
            report.reward_ap = if bonus.event { 0 } else { bonus.ap };
            report.reward_ap_lines = party
                .iter()
                .zip(reward.shares)
                .filter_map(|(fighter, share)| {
                    let ap = share.ap?;
                    let (equipped, learnable) = self.equipment.abilities(fighter.character);
                    let goals = equipped
                        .into_iter()
                        .filter(|&ability| ability != 0)
                        .filter_map(|ability| {
                            learnable
                                .iter()
                                .find(|entry| entry.id == ability)
                                .map(|entry| (ability, entry.max_ap))
                        })
                        .collect();
                    Some((fighter.character, ap, goals))
                })
                .collect();
            report.reward_items = reward.items;
            report.reward_card = reward.card;
        }
        let describe = |id: &usize| {
            let slot = &machine.cmds.slots[*id];
            format!("{}:{}", slot.cmd_no, slot.sub_no)
        };
        report
            .ai_notes
            .push(format!("ready {:?} sent {sent}", machine.ready_players));
        report.ai_notes.push(format!(
            "running [{}] queued [{}]",
            machine
                .running
                .iter()
                .map(describe)
                .collect::<Vec<_>>()
                .join(" "),
            machine
                .cmds
                .queue
                .iter()
                .map(describe)
                .collect::<Vec<_>>()
                .join(" ")
        ));
        let hooks = adapter.hooks();
        report.ai_error = hooks.error.as_ref().map(ToString::to_string);
        if let Some(map) = &hooks.driver.state.map {
            let nonzero: Vec<String> = map
                .bytes()
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte != 0)
                .map(|(index, byte)| format!("{index}={byte}"))
                .collect();
            report
                .ai_notes
                .push(format!("map vars {}", nonzero.join(" ")));
        }
        let objects: Vec<String> = (0..hooks.driver.object_count())
            .map(|slot| {
                format!(
                    "{:?}@{:?}",
                    hooks.driver.object_entry(slot),
                    hooks.driver.object_level(slot)
                )
            })
            .collect();
        report
            .ai_notes
            .push(format!("objects {}", objects.join(" ")));
        for event in &hooks.driver.state.events {
            let note = match event {
                AiEvent::UnmodeledRead { what, id } => format!("unmodeled {what} {id}"),
                AiEvent::UnmodeledSpawn { source } => format!("unmodeled spawn {source}"),
                AiEvent::BattleCode { code, value } => format!("battle code {code}={value}"),
                _ => continue,
            };
            if !report.ai_notes.contains(&note) {
                report.ai_notes.push(note);
            }
        }
        Ok(())
    }
}

/// `btl_sys.SavePlayerData`: each party member's battle HP and MP go back
/// to the player record, clamped to the record's maximum. A fight that
/// failed to run leaves the records alone.
pub fn save_party(report: &FightReport, stats: &mut BTreeMap<i32, CharacterStats>) {
    if report.error.is_some() || report.outcome.is_none() {
        return;
    }
    for &(character, hp, _) in &report.party_hp {
        if let Some(record) = stats.get_mut(&character) {
            record.current_hp = (hp as i32).min(record.max_hp);
        }
    }
    for &(character, mp, _) in &report.party_mp {
        if let Some(record) = stats.get_mut(&character) {
            record.current_mp = (mp as i32).min(record.max_mp);
        }
    }
}

/// The `PLAYER` fields `BattleReward::from_bonus` reads: who is enabled
/// (alive) and still below the EXP cap. The field does not track EXP per
/// level yet, so every line counts as under the cap.
fn reward_member(fighter: &PartyFighter, hp: u32, mp: u32, equipment: &Equipment) -> PartyMember {
    let (equipped_abilities, learnable) = equipment.abilities(fighter.character);
    let stats = fighter.base;
    let base = tantalus_legacy::battle::rewards::BaseStats {
        dex: stats.speed,
        str_: stats.strength,
        mgc: stats.magic,
        wpr: stats.spirit,
        gems: u8::try_from(stats.gems).unwrap_or(u8::MAX),
    };
    let basis = [base.dex, base.str_, base.mgc, base.wpr];
    PartyMember {
        in_battle: true,
        cur_hp: hp,
        cur_mp: mp,
        rewards_disabled: false,
        level: 1,
        exp: 0,
        base,
        bonus: LevelBonus::default(),
        basis,
        max_hp: fighter.max_hp,
        max_mp: fighter.max_mp,
        max_capa: u32::from(base.gems),
        cur_capa: u32::from(base.gems),
        elem: basis,
        equip_stat_bonus: [0; 4],
        pa: vec![0; learnable.len()],
        equipped_abilities,
        learnable,
    }
}

/// `BattleResultUI` pay-out into the field world: gil (`UpdateGil`), item
/// stacks (`UpdateItem`, legacy ids), each line's EXP share, AP toward
/// each equipped learnable ability (`AddAp`, once per item carrying it),
/// and the card (`MiniGame_SetCard`, while the collection has room).
pub fn pay_rewards(report: &FightReport, world: &mut FieldWorld) {
    for &(item, used) in &report.items_used {
        world.add_item_amount(item, -(used.min(99) as i32));
        if world.items.get(&item).is_some_and(|&count| count <= 0) {
            world.items.remove(&item);
        }
    }
    if report.outcome.as_deref() != Some("victory") {
        return;
    }
    world.add_gil(report.reward_gil);
    for &(item, count) in &report.reward_items {
        if let (Ok(item), Ok(count)) = (i32::try_from(item), i32::try_from(count)) {
            world.add_item_amount(item, count);
        }
    }
    for &(character, exp) in &report.reward_exp {
        world.apply_experience(character, u64::from(exp));
    }
    for (character, ap, goals) in &report.reward_ap_lines {
        for &(ability, max_ap) in goals {
            world.apply_ap(*character, ability, max_ap, i32::from(*ap));
        }
    }
    if let Some(card) = report.reward_card {
        world.add_card(card);
    }
}

fn outcome_name(outcome: BattleOutcome) -> &'static str {
    match outcome {
        BattleOutcome::Victory => "victory",
        BattleOutcome::Defeat => "defeat",
        BattleOutcome::Escape => "escape",
        BattleOutcome::EnemyFlee => "enemy_flee",
    }
}

/// A party member at level 1: base stats, the field HP/MP and the default
/// equipment (a power-12 short-range weapon when the tables have none).
fn party_unit(slot: usize, fighter: &PartyFighter, equipment: &Equipment) -> BattleUnit {
    let (weapon, defence) = equipment.outfit(fighter.character);
    let mut unit = BattleUnit::new(1 << slot, true);
    unit.character_id = fighter.character as u8;
    unit.serial_no = tantalus_legacy::battle::unit::default_serial_number(
        unit.character_id,
        equipment.weapon_shape(fighter.character),
        fighter.globals.scenario,
    )
    .unwrap_or(255);
    // `btl_init.cs:371-379`: no gauge before the scenario turns it on, for
    // category 16 (guest characters) or for Garnet while she is depressed.
    let category = tantalus_legacy::battle::unit::default_player_category(unit.character_id);
    unit.has_trance_gauge = fighter.globals.trance_gauge != 0
        && category.is_some_and(|category| category & 16 == 0)
        && !(fighter.character == 2 && fighter.globals.garnet_depress != 0);
    unit.level = 1;
    unit.cur_hp = fighter.hp.max(1);
    unit.max_hp = fighter.max_hp.max(1);
    unit.cur_mp = fighter.mp;
    unit.max_mp = fighter.max_mp;
    // `FF9Play_Update`: `elem = basis + equipment bonus`, capped at 99.
    let bonus = equipment.stat_bonus(fighter.character);
    let stat = |base: u8, add: i32| (i32::from(base) + add).clamp(0, 99) as u8;
    unit.dexterity = stat(fighter.base.speed, bonus[0]);
    unit.strength = stat(fighter.base.strength, bonus[1]);
    unit.magic = stat(fighter.base.magic, bonus[2]);
    unit.will = stat(fighter.base.spirit, bonus[3]).max(1);
    unit.physical_defence = defence[0];
    unit.physical_evade = defence[1];
    unit.magic_defence = defence[2];
    unit.magic_evade = defence[3];
    unit.weapon = Some(weapon.unwrap_or(WeaponStats {
        power: 12,
        rate: 0,
        elements: Elements::NONE,
        category: WeaponStats::SHORT_RANGE,
        status: 0,
    }));
    unit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_bonus_sums_the_bonus_rows_of_the_default_set() {
        let mut equipment = Equipment::default();
        equipment.sets.insert(0, 7);
        equipment.items.insert(7, [1, 89, -1, 100, -1]);
        equipment.bonus_ids.extend([(1, 0), (89, 2), (100, 5)]);
        equipment
            .bonuses
            .extend([(0, [0; 4]), (2, [0, 0, 0, 1]), (5, [1, 2, 0, 0])]);
        assert_eq!(equipment.stat_bonus(0), [1, 2, 0, 1]);
        assert_eq!(equipment.stat_bonus(3), [0; 4]);
    }

    /// Reads the Memoria tables when `TANTALUS_MEMORIA` points at the
    /// reference checkout.
    #[test]
    fn pay_rewards_applies_ap_and_cards() {
        let mut world = FieldWorld::new();
        let report = FightReport {
            outcome: Some("victory".to_owned()),
            reward_ap_lines: vec![(0, 25, vec![(101, 40), (102, 40), (101, 40)])],
            reward_card: Some(7),
            ..FightReport::default()
        };
        pay_rewards(&report, &mut world);
        // An ability on two items gains twice: 25 + 25 reaches 40.
        assert!(world.learned_abilities[&0].contains(&101));
        assert_eq!(world.ability_pa[&0][&102], 25);
        assert_eq!(world.cards, vec![7]);
    }

    #[test]
    fn stat_bonus_reads_memoria_tables_when_present() {
        let Some(root) = std::env::var_os("TANTALUS_MEMORIA") else {
            return;
        };
        let characters = Path::new(&root).join("Memoria.Patcher/StreamingAssets/Data/Characters");
        let equipment = Equipment::load(&characters).expect("Memoria tables load");
        assert!(!equipment.bonuses.is_empty());
        assert!(!equipment.bonus_ids.is_empty());
        // Beatrix's default set carries stat bonuses; Zidane's starting gear
        // carries none.
        assert_eq!(equipment.stat_bonus(11), [1, 2, 3, 8]);
        assert_eq!(equipment.stat_bonus(0), [0; 4]);
        // Zidane starts with a Dagger (Flee, 40 AP to learn).
        let (equipped, learnable) = equipment.abilities(0);
        assert!(equipped.contains(&101), "{equipped:?}");
        assert!(learnable.contains(&PaEntry {
            id: 101,
            max_ap: 40
        }));
        assert_eq!(ability_id("SA:17"), Some(209));
    }

    #[test]
    fn pay_rewards_adds_gil_items_and_exp_on_victory_only() {
        let mut world = FieldWorld::new();
        let mut report = FightReport {
            outcome: Some("defeat".to_owned()),
            reward_gil: 436,
            reward_items: vec![(240, 2)],
            reward_exp: vec![(0, 12)],
            ..FightReport::default()
        };
        pay_rewards(&report, &mut world);
        assert_eq!(world.gil, 0);
        report.outcome = Some("victory".to_owned());
        pay_rewards(&report, &mut world);
        assert_eq!(world.gil, 436);
        assert_eq!(world.items.get(&240), Some(&2));
        assert_eq!(world.character_exp.get(&0), Some(&12));
    }

    #[test]
    fn save_party_copies_battle_hp_and_mp_clamped_to_max() {
        let mut stats = BTreeMap::from([(
            0,
            CharacterStats {
                current_hp: 105,
                max_hp: 105,
                current_mp: 36,
                max_mp: 36,
            },
        )]);
        let report = FightReport {
            outcome: Some("victory".to_owned()),
            party_hp: vec![(0, 40, 105)],
            party_mp: vec![(0, 99, 36)],
            ..FightReport::default()
        };
        save_party(&report, &mut stats);
        assert_eq!(stats[&0].current_hp, 40);
        assert_eq!(stats[&0].current_mp, 36);

        // A fight that did not run leaves the record alone.
        let failed = FightReport {
            error: Some("no scene".to_owned()),
            party_hp: vec![(0, 1, 105)],
            ..FightReport::default()
        };
        save_party(&failed, &mut stats);
        assert_eq!(stats[&0].current_hp, 40);
    }

    #[test]
    fn status_sets_and_serial_numbers() {
        assert_eq!(status_mask(""), 0);
        assert_eq!(status_mask("Venom(1), Poison(16)"), (1 << 1) | (1 << 16));
        use tantalus_legacy::battle::unit::default_serial_number as serial;
        // Zidane: dagger (shape 1) 0, sword 1; Garnet: rod 3 / racket 4
        // before scenario 10300, 5 / 6 after; Blank: armor 17 in 1500-1599.
        assert_eq!(serial(0, 1, 0), Some(0));
        assert_eq!(serial(0, 2, 0), Some(1));
        assert_eq!(serial(2, 8, 2000), Some(3));
        assert_eq!(serial(2, 7, 2000), Some(4));
        assert_eq!(serial(2, 7, 10300), Some(6));
        assert_eq!(serial(3, 0, 0), Some(7));
        assert_eq!(serial(10, 0, 1550), Some(17));
        assert_eq!(serial(10, 0, 2000), Some(16));
        assert_eq!(serial(12, 0, 0), None);
    }
}
