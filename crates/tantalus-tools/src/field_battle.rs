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
use tantalus_game::field::world::EncounterRequest;
use tantalus_game::new_game::BaseStats;
use tantalus_legacy::EventBinary;
use tantalus_legacy::battle::host::{apply_unit_views, battle_views};
use tantalus_legacy::battle::machine::{
    ABILITY_ATTACK, Battle, BattleConfig, BattleHooks, EnemyAction, Phase,
};
use tantalus_legacy::battle::queue::{CommandSpec, cmd};
use tantalus_legacy::battle::rng::SplitMix64Random;
use tantalus_legacy::battle::script_host::{
    AiEvent, BattleScriptDriver, BattleScriptState, BattleTimer, EnemyAiHooks, SceneOverlay,
    ScriptTable,
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
    /// Trance gauge each member carries out (`btl_sys.SavePlayerData`).
    #[serde(skip)]
    pub party_trance: Vec<(i32, u8)>,
    /// Summons cast so far (`BattleAchievement` summon flags), when the
    /// fight ran.
    #[serde(skip)]
    pub summons_seen: Option<Vec<i32>>,
    pub enemy_hp: Vec<(u32, u32)>,
    /// Machine phase changes: tick, `phase/menu-off sequence`, enemy HP.
    pub phase_log: Vec<(u64, String, Vec<u32>)>,
    /// `btl_result` once the machine is over.
    pub result_code: Option<u8>,
    /// The scene's `Info.NoGameOver`: a lost fight goes back to the field.
    #[serde(skip)]
    pub no_game_over: bool,
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
    /// Items Steal took, kept whatever the outcome (live battles only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items_stolen: Vec<u32>,
    /// `(character, ability)` Eat taught (`FF9Abil_SetMaster`), kept
    /// whatever the outcome (live battles only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub abilities_eaten: Vec<(i32, i32)>,
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
    /// Character level (`PLAYER.level`), from the total experience.
    pub level: u8,
    /// Event globals the battle setup reads.
    pub globals: PartyGlobals,
    /// 0 back row, 1 front row (`bi.row`); the Order screen sets it.
    pub row: u8,
    /// `PLAYER.trance`, the gauge carried between battles.
    pub trance: u8,
    /// `PLAYER.equip`: weapon, head, wrist, armor, accessory item ids, -1
    /// for an empty slot. `None` fights in the character's default set.
    pub equip: Option<[i32; 5]>,
    /// `PLAYER.sa`: support abilities switched on, bit `n` is ability
    /// `192 + n` (`SupportAbility`).
    pub support: u64,
}

impl PartyFighter {
    /// The gear and enabled support abilities the field world records for
    /// `character` (`PLAYER.equip`, `PLAYER.sa`); no equipment entry means
    /// the default set.
    pub fn loadout(world: &FieldWorld, character: i32) -> (Option<[i32; 5]>, u64) {
        let equip = world
            .equipment
            .get(&character)
            .map(|slots| slots.map(|item| item.map_or(-1, i32::from)));
        let support = world
            .support_enabled
            .get(&character)
            .into_iter()
            .flatten()
            .filter_map(|&ability| u32::try_from(ability - 192).ok())
            .filter(|&bit| bit < 64)
            .fold(0_u64, |mask, bit| mask | 1 << bit);
        (equip, support)
    }
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
    /// `FF9StateSystem.Achievement.summon_*` for this session: summons the
    /// party has cast (`BattleAchievement.UpdateCommandAchievement`). Not
    /// in the save yet.
    summons_seen: std::sync::Mutex<Vec<i32>>,
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
    /// `BonusId` -> element sets bonus, guard, absorb, half, weak
    /// (`Stats.csv` `BonusElement` .. `WeakElement`).
    bonus_elements: BTreeMap<i32, [u8; 5]>,
    /// Weapon id -> `StatusIndex` into `status_sets` (`Weapons.csv`).
    weapon_status_index: BTreeMap<i32, i32>,
    /// Item id -> ability ids (`Items.csv` `AbilityIds`: `AA:X` is X,
    /// `SA:X` is 192 + X, `ff9abil.cs:24-25`).
    item_abilities: BTreeMap<i32, Vec<i32>>,
    /// Model id -> `FF9DBModelParameters` Radius (Memoria
    /// `FF9DBModelParameters.cs`); empty when the file is missing.
    model_radius: BTreeMap<i32, i32>,
    /// Spell and item effect ids and their timing (for drawing).
    effects: std::sync::Arc<crate::battle_effects::EffectTable>,
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
    /// Ability id -> its `Actions.csv` comment name (`Fire`), for menus.
    action_names: BTreeMap<i32, String>,
    /// Preset id -> its `Ability1` and `Ability2` commands, then the two
    /// in Trance (`Characters/CommandSets.csv` `First`, `Second`,
    /// `TranceFirst`, `TranceSecond`).
    command_sets: BTreeMap<i32, [u32; 4]>,
    /// `BattleCommandId` -> its `Characters/Commands.csv` row.
    commands: BTreeMap<u32, CommandRow>,
}

/// `{ 12, new FF9DBModelParameters(height, radius, ...) }` entries: model
/// id -> radius.
fn parse_model_radius(source: &str) -> BTreeMap<i32, i32> {
    let mut radius = BTreeMap::new();
    for line in source.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('{') else {
            continue;
        };
        let Some((id, args)) = rest.split_once(", new FF9DBModelParameters(") else {
            continue;
        };
        let mut numbers = args.split(',').map(|field| field.trim().parse::<i32>());
        let (Ok(id), Some(Ok(_height)), Some(Ok(value))) =
            (id.trim().parse::<i32>(), numbers.next(), numbers.next())
        else {
            continue;
        };
        radius.insert(id, value);
    }
    radius
}

/// A `Commands.csv` row (`CharacterCommand`): type 0 runs `main` directly,
/// 1 lists abilities, 2 is Item, 3 Throw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CommandRow {
    kind: u8,
    main: i32,
    list: Vec<i32>,
    name: String,
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
    /// Item id -> weapon power, for Throw (`Weapon.Find(item).Power`).
    weapon_power: BTreeMap<i32, i32>,
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
                _ if ABILITY_COMMANDS.contains(&cmd_no) => self.player_spec(cmd_no, sub_no),
                // `aa_data[Throw]`; the machine's `throw_spec` adds the
                // item's weapon power.
                cmd::THROW => self.player_spec(cmd_no, ABILITY_THROW),
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
    fn item_weapon_power(&mut self, item_id: i32) -> Option<i32> {
        self.weapon_power.get(&item_id).copied()
    }

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
    /// Times every spell and item effect on the translated plugin
    /// ([`crate::battle_effects::EffectTable::load_plugin_timing`]).
    pub fn load_plugin_timing(&mut self, game_data: &Path) {
        std::sync::Arc::make_mut(&mut self.effects).load_plugin_timing(game_data);
    }

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
        let mut effect_table = crate::battle_effects::EffectTable::default();
        effect_table.load_timing(&characters.join("../SpecialEffects"));
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
            if let Some(index) = int(&row, 3) {
                equipment.weapon_status_index.insert(id, index);
            }
            equipment.weapons.insert(
                id,
                WeaponStats {
                    power,
                    rate: int(&row, 8).unwrap_or(0),
                    elements: Elements(row.get(7).map_or(0, |list| status_mask(list)) as u8),
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
            // BonusElement;GuardElement;AbsorbElement;HalfElement;WeakElement
            let mut sets = [0_u8; 5];
            for (index, set) in sets.iter_mut().enumerate() {
                *set = row.get(6 + index).map_or(0, |list| status_mask(list)) as u8;
            }
            equipment.bonus_elements.insert(id, sets);
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
            let animations: BTreeMap<i32, i32> = csv_rows(&effects)
                .filter_map(|row| Some((int(&row, 0)?, int(&row, 4)?)))
                .collect();
            for row in csv_rows(&items) {
                if let (Some(id), Some(effect)) = (int(&row, 0), int(&row, 3))
                    && let Some(item) = rows.get(&effect)
                {
                    equipment.item_effects.insert(id, *item);
                    if let Some(animation) = animations.get(&effect) {
                        effect_table.items.insert(id, *animation);
                    }
                }
            }
        }
        // Comment;Id;Display;Targets;DefaultAlly;ForDead;DefaultDead;
        // DefaultCamera;AnimationId;AnimationId2;ScriptId;Power;Elements;
        // Rate;Category;AddStatus;MP;Type;...
        if let Ok(text) = read(characters.join("CommandSets.csv")) {
            for row in csv_rows(&text) {
                let (Some(id), Some(first), Some(second)) =
                    (int(&row, 0), int(&row, 3), int(&row, 4))
                else {
                    continue;
                };
                let trance_first = int(&row, 9).unwrap_or(first);
                let trance_second = int(&row, 10).unwrap_or(second);
                equipment.command_sets.insert(
                    id,
                    [first, second, trance_first, trance_second].map(|cmd_no| cmd_no.max(0) as u32),
                );
            }
        }
        if let Ok(text) = read(characters.join("Commands.csv")) {
            for row in csv_rows(&text) {
                let Some(id) = int(&row, 0) else { continue };
                let field = |index: usize| row.get(index).copied().unwrap_or("");
                equipment.commands.insert(
                    id.max(0) as u32,
                    CommandRow {
                        kind: int(&row, 1).unwrap_or(0).clamp(0, 255) as u8,
                        main: int(&row, 2).unwrap_or(0),
                        list: field(3)
                            .split(',')
                            .filter_map(|id| id.trim().parse().ok())
                            .collect(),
                        name: field(4).trim().trim_start_matches('#').trim().to_owned(),
                    },
                );
            }
        }
        if let Ok(text) = read(characters.join("../Battle/Actions.csv")) {
            for row in csv_rows(&text) {
                let Some(id) = int(&row, 1) else { continue };
                let field = |index: usize| row.get(index).copied().unwrap_or("");
                let flag = |index: usize| matches!(field(index).trim(), "1" | "True" | "true");
                let byte = |index: usize| int(&row, index).unwrap_or(0).clamp(0, 255) as u8;
                equipment
                    .action_names
                    .insert(id, field(0).trim().to_owned());
                if let (Some(many), Some(single)) = (int(&row, 8), int(&row, 9)) {
                    let many_any = paren_or_number(field(3)) == "3";
                    effect_table.abilities.insert(id, (many, single, many_any));
                }
                equipment.actions.insert(
                    id,
                    ActionRow {
                        target: paren_or_number(field(3)).parse().unwrap_or(0),
                        default_ally: flag(4),
                        for_dead: flag(5),
                        script_id: vanilla_script_id(id, int(&row, 10).unwrap_or(0)),
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
        equipment.effects = std::sync::Arc::new(effect_table);
        // `Memoria.Patcher/StreamingAssets/Data/Characters` -> the root.
        if let Ok(source) =
            read(characters.join("../../../../Assembly-CSharp/Global/ff9/FF9DBModelParameters.cs"))
        {
            equipment.model_radius = parse_model_radius(&source);
        }
        Ok(equipment)
    }

    /// The five items a member fights with: what the field world carries
    /// (`PLAYER.equip`), else the character's default set. -1 is an empty
    /// slot.
    fn slots(&self, fighter: &PartyFighter) -> Option<[i32; 5]> {
        fighter
            .equip
            .or_else(|| self.default_slots(fighter.character))
    }

    fn default_slots(&self, character: i32) -> Option<[i32; 5]> {
        self.sets
            .get(&character)
            .and_then(|set| self.items.get(set))
            .copied()
    }

    /// The weapon's `GraphicsId` (`WeaponShape`), 0 without one.
    fn weapon_shape(&self, slots: Option<[i32; 5]>) -> u8 {
        slots
            .and_then(|slots| self.shapes.get(&slots[0]))
            .copied()
            .unwrap_or(0)
    }

    /// The equipped weapon (with its element and added status set,
    /// `BattleUnit.cs:225-229`) and summed armor defences, capped at 255
    /// (`FF9PLAY_DEFPARAM_VAL_MAX`, `ff9play.FF9Play_Update`).
    fn outfit(&self, slots: Option<[i32; 5]>) -> (Option<WeaponStats>, [i32; 4]) {
        let Some(slots) = slots else {
            return (None, [0; 4]);
        };
        let weapon = self.weapons.get(&slots[0]).map(|stats| {
            let mut stats = *stats;
            stats.status = self
                .weapon_status_index
                .get(&slots[0])
                .and_then(|index| self.status_sets.get(index))
                .copied()
                .unwrap_or(0);
            stats
        });
        let mut defence = [0; 4];
        for &item in &slots[1..] {
            if let Some(stats) = self.armors.get(&(item - ARMOR_BASE)) {
                for (total, value) in defence.iter_mut().zip(stats) {
                    *total += value;
                }
            }
        }
        for value in &mut defence {
            *value = (*value).min(255);
        }
        (weapon, defence)
    }

    /// `btl_eqp.InitEquipPrivilegeAttrib`: bonus, guard, absorb, half and
    /// weak element sets, the union over the five items.
    fn element_sets(&self, slots: Option<[i32; 5]>) -> [u8; 5] {
        let mut total = [0_u8; 5];
        for item in slots.iter().flatten() {
            if let Some(sets) = self
                .bonus_ids
                .get(item)
                .and_then(|bonus| self.bonus_elements.get(bonus))
            {
                for (sum, value) in total.iter_mut().zip(sets) {
                    *sum |= value;
                }
            }
        }
        total
    }

    /// `itemData.ability` of every equipped item, slot order, and the
    /// character's learnable table (`_FF9Abil_PaData[PresetId]`).
    fn abilities(&self, character: i32, slots: Option<[i32; 5]>) -> (Vec<i32>, Vec<PaEntry>) {
        let equipped = slots
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
    fn stat_bonus(&self, slots: Option<[i32; 5]>) -> [i32; 4] {
        let mut total = [0; 4];
        for item in slots.iter().flatten() {
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
            summons_seen: std::sync::Mutex::new(Vec::new()),
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
            summons_seen: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Sets the summons later fights start from (the saved game's).
    pub fn set_summons_seen(&self, seen: &[i32]) {
        if let Ok(mut current) = self.summons_seen.lock() {
            current.clear();
            current.extend_from_slice(seen);
        }
    }

    /// Fights `request` with `party`. [`save_party`] and [`pay_rewards`] carry
    /// the result back to the field.
    pub fn fight(&self, request: &EncounterRequest, party: &[PartyFighter]) -> FightReport {
        self.fight_with_items(request, party, &BTreeMap::new())
    }

    /// [`Self::fight`] with the field inventory (`item id -> count`), which
    /// the Item command reads. The report lists what was used. The stand-in
    /// party policy picks every party command; the whole fight runs here.
    pub fn fight_with_items(
        &self,
        request: &EncounterRequest,
        party: &[PartyFighter],
        items: &BTreeMap<i32, i32>,
    ) -> FightReport {
        match self.start(request, party, items) {
            Ok(mut fight) => {
                while fight.step(FightControl::StandIn) {}
                fight.finish(self)
            }
            Err(report) => *report,
        }
    }

    /// Sets up `request` to be fought one battle tick at a time
    /// ([`LiveFight::step`]), the way a player fights it. A battle that
    /// cannot start comes back as its report, with `error` set.
    pub fn start(
        &self,
        request: &EncounterRequest,
        party: &[PartyFighter],
        items: &BTreeMap<i32, i32>,
    ) -> Result<LiveFight, Box<FightReport>> {
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
        match self.run(request, &mut report) {
            Ok((scene, sequence, pattern, script)) => {
                let timer = request.timer.map(|timer| BattleTimer {
                    time: timer.time(),
                    running: timer.running,
                });
                self.battle(
                    scene, sequence, pattern, script, party, items, timer, report,
                )
            }
            Err(error) => Err(fail(report, error)),
        }
    }

    /// The scene, its sequence file, the pattern and the AI script of
    /// `request`.
    #[allow(clippy::type_complexity)]
    fn run(
        &self,
        request: &EncounterRequest,
        report: &mut FightReport,
    ) -> Result<(&BattleScene, Option<&[u8]>, usize, Option<&[u8]>), String> {
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
        let sequence = self.data.sequence_bytes.get(&name).map(Vec::as_slice);
        Ok((scene, sequence, pattern, script.map(Vec::as_slice)))
    }

    #[allow(clippy::too_many_arguments)]
    fn battle(
        &self,
        scene: &BattleScene,
        sequence: Option<&[u8]>,
        pattern: usize,
        script: Option<&[u8]>,
        party: &[PartyFighter],
        items: &BTreeMap<i32, i32>,
        timer: Option<BattleTimer>,
        mut report: FightReport,
    ) -> Result<LiveFight, Box<FightReport>> {
        // `SB2_HEAD` flag 4 is `Info.NoGameOver` (`BTL_SCENE.cs`).
        report.no_game_over = scene.header.flags & 0x4 != 0;
        let mut units: Vec<BattleUnit> = party
            .iter()
            .enumerate()
            .map(|(slot, fighter)| party_unit(slot, fighter, &self.equipment))
            .collect();
        let mut pairs: Vec<(EntityId, u16)> = units
            .iter()
            .map(|unit| (EntityId::new(1000 + u64::from(unit.id)), unit.id))
            .collect();
        let Some(enemies) = pattern_machine_units(scene, pattern) else {
            return Err(fail(
                report,
                format!("pattern {pattern} has no machine units"),
            ));
        };
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
                command_timing_late: {
                    let table = self.equipment.effects.clone();
                    // Enemy attacks play the effect their attack sequence
                    // starts (`SeqExecVfx` / `SeqExecSVfx`), not the scene
                    // entry's `VfxIndex`; a sequence with no effect (plain
                    // melee) keeps the flat wait.
                    let attack_effects: Vec<Option<i32>> = sequence
                        .map(crate::battle_stage::attack_vfx)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|vfx| vfx.map(|vfx| i32::from(vfx.effect)))
                        .collect();
                    tantalus_legacy::battle::machine::LateTimings(Some(std::sync::Arc::new(
                        move |key| {
                            if key.2 == tantalus_legacy::battle::machine::ENEMY_ATTACK_VARIANT {
                                let effect =
                                    (*attack_effects.get(usize::try_from(key.1).ok()?)?)?;
                                return table.effect_timing_ticks(effect, false);
                            }
                            table.command_timing_of(key)
                        },
                    )))
                },
                cast_leads: std::sync::Arc::new(crate::battle_effects::cast_leads()),
                summons_seen: self
                    .summons_seen
                    .lock()
                    .map(|seen| seen.clone())
                    .unwrap_or_default(),
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
            Some(bytes) => match EventBinary::parse(bytes) {
                Ok(binary) => ScriptTable::from_binary(&binary),
                Err(error) => return Err(fail(report, error.to_string())),
            },
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
        let ids = match StandInActions::new() {
            Ok(ids) => ids,
            Err(error) => return Err(fail(report, error)),
        };
        // Abilities each member's default gear gives (`AA:X` ids under 192).
        let gear: Vec<Vec<i32>> = party
            .iter()
            .map(|fighter| {
                self.equipment
                    .abilities(fighter.character, self.equipment.slots(fighter))
                    .0
            })
            .collect();
        let mut action_map = BTreeMap::from([
            (ids.attack.clone(), (cmd::ATTACK, ABILITY_ATTACK)),
            (ids.fire.clone(), (BLACK_MAGIC, ABILITY_FIRE)),
            (ids.free_energy.clone(), (INSIDE_TRICK, ABILITY_FREE_ENERGY)),
            (ids.potion.clone(), (cmd::ITEM, ITEM_POTION)),
            (ids.hi_potion.clone(), (cmd::ITEM, ITEM_HI_POTION)),
            (ids.pinion.clone(), (cmd::ITEM, ITEM_PHOENIX_PINION)),
            (ids.remedy.clone(), (cmd::ITEM, ITEM_REMEDY)),
            (ids.phoenix.clone(), (cmd::ITEM, ITEM_PHOENIX_DOWN)),
        ]);
        // What the command menu can send: Defend, each member's gear
        // abilities under the member's ability command, and every usable
        // item. The stand-in policy never sends these, and the map is only
        // looked up, so the stand-in fights are unchanged.
        let menu = player_menus(party, &gear, &self.equipment, items);
        for entry in menu.iter().flat_map(|menu| menu.entries()) {
            action_map.insert(entry.action.clone(), (entry.cmd_no, entry.sub_no));
        }
        let adapter = LegacyBattleAdapter::new(battle, action_map, unit_map, target_bits);
        let hooks = EnemyAiHooks::new(
            SceneAttacks {
                attacks: scene.attacks.clone(),
                status_sets: self.equipment.status_sets.clone(),
                actions: self.equipment.actions.clone(),
                item_effects: self.equipment.item_effects.clone(),
                weapon_power: self
                    .equipment
                    .weapons
                    .iter()
                    .map(|(&id, weapon)| (id, weapon.power))
                    .collect(),
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
                timer,
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
        stage.attack_anims = sequence
            .map(crate::battle_stage::attack_animations)
            .unwrap_or_default();
        stage.attack_turns = sequence
            .map(crate::battle_stage::attack_turns)
            .unwrap_or_default();
        stage.sfx = sequence
            .map(crate::battle_sound::sequence_sounds)
            .unwrap_or_default();
        stage.enemy_radius = stage
            .units
            .iter()
            .find_map(|unit| match unit.model {
                crate::battle_stage::StageModel::Enemy { geo, .. } => Some(geo),
                _ => None,
            })
            .and_then(|geo| self.equipment.model_radius.get(&i32::from(geo)).copied())
            .unwrap_or(0);
        stage.effects = self.equipment.effects.clone();
        stage.attack_effects = sequence
            .map(crate::battle_stage::attack_vfx)
            .unwrap_or_default();
        stage.camera_block =
            sequence.and_then(tantalus_assets::pc::battle_camera::CameraBlock::from_raw17);
        stage.attack_cameras = sequence
            .map(crate::battle_stage::attack_cameras)
            .unwrap_or_default();
        // The attacks whose camera is `RunCamera` 0x12: the machine draws
        // `ShouldPlayAlternateCamera` at the op's frame.
        adapter.battle_mut().config.alternate_cameras = std::sync::Arc::new(
            stage
                .attack_cameras
                .iter()
                .enumerate()
                .filter_map(|(index, camera)| {
                    let camera = camera.filter(|camera| camera.alternate)?;
                    let default_camera = scene
                        .attacks
                        .get(index)
                        .is_some_and(|attack| attack.default_camera);
                    Some((i32::try_from(index).ok()?, (camera.frame, default_camera)))
                })
                .collect(),
        );
        // `btl_sys.SetBonus` on each enemy death; `Info.AfterEvent` scenes
        // pay nothing (`battle.cs:72`).
        let rows = pattern_rewards(scene, pattern).unwrap_or_default();
        let pattern_ap = scene.patterns.get(pattern).map_or(0, |row| row.ap);
        let member_flag = (1u8 << party.len().min(4)) - 1;
        let bonus = BattleBonus::new(pattern_ap, member_flag, scene.header.flags & 2048 != 0);
        let draws = SplitMix64Random::new(u64::from(report.battle as u32) ^ 0xD1AA);
        let hp = adapter
            .battle()
            .units
            .iter()
            .map(|unit| unit.cur_hp)
            .collect();
        Ok(LiveFight {
            report,
            party: party.to_vec(),
            items: items.clone(),
            entities,
            gear,
            ids,
            trance_menu: menu
                .iter()
                .map(|menu| PartyMenu {
                    commands: menu.trance_commands.clone(),
                    ..menu.clone()
                })
                .collect(),
            menu,
            adapter,
            world: FieldWorld::new(),
            defeated: Vec::new(),
            rows,
            bonus,
            draws,
            events_seen: 0,
            sent: 0,
            countered: false,
            stage,
            tick: 0,
            log_seen: 0,
            hp,
            state: FightState::Running,
        })
    }
}

/// A fight that could not start: its report with `error` set.
fn fail(mut report: FightReport, error: String) -> Box<FightReport> {
    report.error = Some(error);
    Box::new(report)
}

/// Who picks the party's commands for one [`LiveFight::step`].
#[derive(Debug, Clone, PartialEq)]
pub enum FightControl {
    /// The stand-in policy (the headless chain and sweeps).
    StandIn,
    /// A player: the command to queue this tick, if any.
    Player(Option<BattleCommand>),
}

#[derive(Debug, Clone, PartialEq)]
enum FightState {
    Running,
    Ended,
    Failed(String),
}

/// Fight length after which the stand-in policy attacks the weakest enemy first.
const STALEMATE_TICKS: u64 = 1500;

/// The action ids the stand-in policy sends.
#[derive(Debug, Clone)]
struct StandInActions {
    attack: ContentId,
    fire: ContentId,
    free_energy: ContentId,
    potion: ContentId,
    hi_potion: ContentId,
    pinion: ContentId,
    remedy: ContentId,
    phoenix: ContentId,
}

impl StandInActions {
    fn new() -> Result<Self, String> {
        let id = |text: &str| ContentId::parse(text).map_err(|e| e.to_string());
        Ok(Self {
            attack: id("tantalus:action/attack")?,
            fire: id("tantalus:action/fire")?,
            free_energy: id("tantalus:action/free-energy")?,
            potion: id("tantalus:action/potion")?,
            hi_potion: id("tantalus:action/hi-potion")?,
            pinion: id("tantalus:action/phoenix-pinion")?,
            remedy: id("tantalus:action/remedy")?,
            phoenix: id("tantalus:action/phoenix-down")?,
        })
    }
}

/// Which side a command aims at first (`DefaultAlly`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAim {
    Enemies,
    Allies,
    /// Defend: the caster alone.
    Caster,
}

/// One command a party member can pick.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuEntry {
    /// `Actions.csv` name, or the item id as text (a client names items).
    pub label: String,
    pub action: ContentId,
    pub cmd_no: u32,
    pub sub_no: i32,
    pub aim: MenuAim,
    /// `Actions.csv` `Targets` (`TargetType`): whether the cursor can
    /// cover a whole side ([`MenuEntry::can_toggle_all`]) or must.
    pub target: u8,
    /// MP the ability costs; 0 for items and basic commands.
    pub mp: u32,
    /// Item id for Item entries.
    pub item: Option<i32>,
    /// Can target a fallen ally (`ForDead`).
    pub for_dead: bool,
    /// False for commands whose effect script is not ported yet; the menu
    /// shows them grayed out.
    pub enabled: bool,
}

impl MenuEntry {
    /// `ManyAny` / `ManyAlly` / `ManyEnemy`: R1 switches between one
    /// target and the whole side (`BattleHUD` `TargetPanel` toggle).
    pub fn can_toggle_all(&self) -> bool {
        matches!(
            self.target,
            target_type::MANY_ANY | target_type::MANY_ALLY | target_type::MANY_ENEMY
        )
    }

    /// `All*` and `Everyone`: the cursor always covers the side.
    pub fn always_all(&self) -> bool {
        matches!(
            self.target,
            target_type::ALL
                | target_type::ALL_ALLY
                | target_type::ALL_ENEMY
                | target_type::EVERYONE
        )
    }
}

/// What an ability command row opens.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandKind {
    /// Goes straight to the target cursor (Steal).
    Direct(MenuEntry),
    /// An ability list (Skill, Black Magic, Sword Art, ...).
    List(Vec<MenuEntry>),
    /// A command the menu cannot run yet (Jump, Eat, Throw, ...), shown
    /// grayed out.
    Unavailable,
}

/// One of the two ability command rows (`Ability1`, `Ability2`).
#[derive(Debug, Clone, PartialEq)]
pub struct MenuCommand {
    /// `Commands.csv` name (`Steal`, `Skill`, `Blk Mag`).
    pub title: String,
    pub cmd_no: u32,
    pub kind: CommandKind,
}

/// A party member's command menu (`BattleHUD.DisplayCommand`): Attack, the
/// member's two ability commands from `CommandSets.csv`, Item, and Defend
/// and Change beside them.
#[derive(Debug, Clone, PartialEq)]
pub struct PartyMenu {
    pub attack: MenuEntry,
    pub commands: Vec<MenuCommand>,
    /// The two ability commands in Trance (Zidane's Dyne for Skill).
    pub trance_commands: Vec<MenuCommand>,
    pub defend: MenuEntry,
    pub change: MenuEntry,
    pub items: Vec<MenuEntry>,
}

impl PartyMenu {
    fn entries(&self) -> impl Iterator<Item = &MenuEntry> {
        let commands = self
            .commands
            .iter()
            .chain(&self.trance_commands)
            .flat_map(|command| match &command.kind {
                CommandKind::Direct(entry) => std::slice::from_ref(entry),
                CommandKind::List(list) => list.as_slice(),
                CommandKind::Unavailable => &[],
            });
        [&self.attack, &self.defend, &self.change]
            .into_iter()
            .chain(commands)
            .chain(&self.items)
    }
}

/// `CharacterId` -> its default preset (`CharacterPresetId`), the row in
/// `CommandSets.csv` (Marcus, Blank and Beatrix have two presets each).
fn preset(character: i32) -> i32 {
    match character {
        9 => 10,
        10 => 12,
        11 => 14,
        other => other,
    }
}

/// `CharacterCommands` default command 2 of each `CharacterId`
/// (`BattleCommandId`) and its menu name, for a preset without a
/// `CommandSets.csv` row.
fn ability_command(character: i32) -> (u32, &'static str) {
    match character {
        0 => (25, "Skill"),
        1 => (BLACK_MAGIC, "Black Mag"),
        2 => (17, "White Mag"),
        3 => (30, "Sword Art"),
        4 => (27, "Dragon"),
        5 => (24, "Blu Mag"),
        6 => (19, "White Mag"),
        7 => (28, "Flair"),
        11 => (34, "White Mag"),
        _ => (25, "Skill"),
    }
}

const STEAL: u32 = 2;
const CHANGE: u32 = 7;
const ABILITY_DEFEND: i32 = 179;
const ABILITY_CHANGE: i32 = 182;
/// Effect scripts of direct commands the machine runs (58 Steal, 65 Eat
/// and Cook).
const DIRECT_SCRIPTS: [i32; 2] = [58, 65];
/// Direct commands the machine runs without an effect script: Jump (the
/// machine's `start_jump`) and Focus (script 44, but on the caster).
const DIRECT_COMMANDS: [u32; 2] = [JUMP, FOCUS];
const JUMP: u32 = 3;
const EAT: u32 = 8;
const COOK: u32 = 9;
const SPEAR: u32 = 10;
const FOCUS: u32 = 13;
/// `Actions.csv` row of the Throw command (script 42).
const ABILITY_THROW: i32 = 190;

/// The player commands whose `sub_no` is an `Actions.csv` row, read like
/// the stand-in's Black Magic and Dyne: Steal, Change and the ability
/// lists (`btl_util.cs:427`).
const ABILITY_COMMANDS: [u32; 26] = [
    STEAL,
    JUMP,
    CHANGE,
    EAT,
    COOK,
    SPEAR,
    FOCUS,
    16,
    17,
    18,
    19,
    20,
    21,
    BLACK_MAGIC,
    23,
    24,
    25,
    INSIDE_TRICK,
    27,
    28,
    29,
    30,
    31,
    32,
    33,
    34,
];

fn player_menus(
    party: &[PartyFighter],
    gear: &[Vec<i32>],
    equipment: &Equipment,
    items: &BTreeMap<i32, i32>,
) -> Vec<PartyMenu> {
    let entry = |label: String, cmd_no: u32, sub_no: i32, aim, target, mp, item, for_dead| {
        ContentId::parse(&format!("tantalus:command/{cmd_no}/{sub_no}"))
            .ok()
            .map(|action| MenuEntry {
                label,
                action,
                cmd_no,
                sub_no,
                aim,
                target,
                mp,
                item,
                for_dead,
                enabled: true,
            })
    };
    // An `Actions.csv` row as a menu entry under `cmd_no`.
    let action = |cmd_no: u32, ability: i32| {
        let row = equipment.actions.get(&ability)?;
        let aim = if row.target >= target_type::SELF {
            MenuAim::Caster
        } else if row.default_ally {
            MenuAim::Allies
        } else {
            MenuAim::Enemies
        };
        let label = equipment
            .action_names
            .get(&ability)
            .cloned()
            .unwrap_or_else(|| ability.to_string());
        entry(
            label,
            cmd_no,
            ability,
            aim,
            row.target,
            row.mp.max(0) as u32,
            None,
            row.for_dead,
        )
    };
    let usable: Vec<MenuEntry> = items
        .iter()
        .filter(|&(_, &count)| count > 0)
        .filter_map(|(&item, _)| {
            let row = equipment.item_effects.get(&item)?;
            let aim = if row.default_ally {
                MenuAim::Allies
            } else {
                MenuAim::Enemies
            };
            entry(
                item.to_string(),
                cmd::ITEM,
                item,
                aim,
                row.target,
                0,
                Some(item),
                row.for_dead,
            )
        })
        .collect();
    party
        .iter()
        .zip(gear)
        .filter_map(|(fighter, gear)| {
            // Only what the member can learn: gear lists every
            // character's abilities (`Abilities/<Preset>.csv`).
            let learnable = equipment
                .abilities(fighter.character, equipment.slots(fighter))
                .1;
            let known: Vec<i32> = gear
                .iter()
                .copied()
                .filter(|&ability| ability > 0 && ability < 192)
                .filter(|&ability| learnable.iter().any(|entry| entry.id == ability))
                .collect();
            let set = equipment
                .command_sets
                .get(&preset(fighter.character))
                .copied()
                .unwrap_or_else(|| {
                    let ability = ability_command(fighter.character).0;
                    [STEAL, ability, STEAL, ability]
                });
            // `BattleHUD.SetAbilityTrance`: entry `j` of a Trance command
            // (Dyne) is known when entry `j` of the command it replaces
            // (Skill) is.
            let mut trance_known = known.clone();
            for (normal, trance) in [(set[0], set[2]), (set[1], set[3])] {
                let lists = equipment
                    .commands
                    .get(&normal)
                    .zip(equipment.commands.get(&trance));
                if let Some((normal, trance)) = lists.filter(|_| normal != trance) {
                    for (from, to) in normal.list.iter().zip(&trance.list) {
                        if from != to && known.contains(from) {
                            trance_known.push(*to);
                        }
                    }
                }
            }
            let build = |set: &[u32], known: &Vec<i32>| -> Vec<MenuCommand> {
                set.iter()
                    .filter(|&&cmd_no| cmd_no != 0)
                    .map(|&cmd_no| {
                        let row = equipment.commands.get(&cmd_no);
                        let title = row
                            .map(|row| row.name.clone())
                            .unwrap_or_else(|| ability_command(fighter.character).1.to_owned());
                        let kind = match row.map(|row| row.kind) {
                            Some(0) => {
                                let main = row.map_or(0, |row| row.main);
                                let ported = DIRECT_COMMANDS.contains(&cmd_no)
                                    || equipment
                                        .actions
                                        .get(&main)
                                        .is_some_and(|row| DIRECT_SCRIPTS.contains(&row.script_id));
                                match action(cmd_no, main) {
                                    Some(entry) if ported => CommandKind::Direct(entry),
                                    _ => CommandKind::Unavailable,
                                }
                            }
                            Some(1) | None => {
                                // The command's list, in its order, of the
                                // abilities the member has.
                                let list: Vec<i32> = match row {
                                    Some(row) => row.list.clone(),
                                    None => known.clone(),
                                };
                                let mut entries: Vec<MenuEntry> = list
                                    .iter()
                                    .filter(|ability| known.contains(ability))
                                    .filter_map(|&ability| action(cmd_no, ability))
                                    .collect();
                                entries.dedup_by_key(|entry| entry.sub_no);
                                CommandKind::List(entries)
                            }
                            // Throw: the weapons in the bag
                            // (`BattleHUD` `ItemPanel` in throw mode).
                            Some(3) => CommandKind::List(
                                items
                                    .iter()
                                    .filter(|&(item, &count)| {
                                        count > 0 && equipment.weapons.contains_key(item)
                                    })
                                    .filter_map(|(&item, _)| {
                                        let mut throw = action(cmd_no, ABILITY_THROW)?;
                                        throw.label = item.to_string();
                                        throw.sub_no = item;
                                        throw.item = Some(item);
                                        throw.action = ContentId::parse(&format!(
                                            "tantalus:command/{cmd_no}/{item}"
                                        ))
                                        .ok()?;
                                        Some(throw)
                                    })
                                    .collect(),
                            ),
                            _ => CommandKind::Unavailable,
                        };
                        MenuCommand {
                            title,
                            cmd_no,
                            kind,
                        }
                    })
                    .collect()
            };
            let commands = build(&set[..2], &known);
            let trance_commands = build(&set[2..], &trance_known);
            Some(PartyMenu {
                attack: entry(
                    "Attack".into(),
                    cmd::ATTACK,
                    ABILITY_ATTACK,
                    MenuAim::Enemies,
                    target_type::SINGLE_ENEMY,
                    0,
                    None,
                    false,
                )?,
                commands,
                trance_commands,
                defend: entry(
                    "Defend".into(),
                    cmd::DEFEND,
                    ABILITY_DEFEND,
                    MenuAim::Caster,
                    target_type::SELF,
                    0,
                    None,
                    false,
                )?,
                change: action(CHANGE, ABILITY_CHANGE).or_else(|| {
                    entry(
                        "Change".into(),
                        CHANGE,
                        ABILITY_CHANGE,
                        MenuAim::Caster,
                        target_type::SELF,
                        0,
                        None,
                        false,
                    )
                })?,
                items: usable.clone(),
            })
        })
        .collect()
}

/// A battle being fought one tick at a time ([`FieldBattles::start`]).
pub struct LiveFight {
    report: FightReport,
    party: Vec<PartyFighter>,
    items: BTreeMap<i32, i32>,
    entities: Vec<EntityId>,
    gear: Vec<Vec<i32>>,
    ids: StandInActions,
    menu: Vec<PartyMenu>,
    /// [`Self::menu`] with each member's Trance commands.
    trance_menu: Vec<PartyMenu>,
    adapter: LegacyBattleAdapter<EnemyAiHooks<SceneAttacks>>,
    world: FieldWorld,
    defeated: Vec<EntityId>,
    rows: Vec<tantalus_legacy::battle::rewards::EnemyRewardData>,
    bonus: BattleBonus,
    draws: SplitMix64Random,
    events_seen: usize,
    sent: u32,
    /// An enemy counter command aimed at the party has run: the stand-in Vivi stops casting.
    countered: bool,
    stage: crate::battle_stage::BattleStage,
    tick: u64,
    /// Read position into the machine's event log, for the stage record.
    log_seen: usize,
    /// HP after the last recorded event, for damage amounts.
    hp: Vec<u32>,
    state: FightState,
}

impl LiveFight {
    /// The machine (units are party then enemies).
    pub fn machine(&self) -> &Battle {
        self.adapter.battle()
    }

    pub fn party_len(&self) -> usize {
        self.party.len()
    }

    /// Applies the Config screen's Battle Speed (`cfg.btl_speed`, 0 to 2) to a
    /// battle that has not started ticking.
    pub fn set_battle_speed(&mut self, speed: u64) {
        self.adapter
            .battle_mut()
            .set_btl_speed(u8::try_from(speed).unwrap_or(1));
    }

    /// Applies the Config screen's battle settings to a battle that has not
    /// started ticking: Battle Speed (`cfg.btl_speed`) and Battle Camera
    /// (`cfg.camera`, 1 = Fixed: `RecomputeGate323268` asks the host
    /// `GetCameraConfig` and turns the effects' own cameras off).
    pub fn apply_settings(&mut self, settings: &tantalus_game::field::settings::GameSettings) {
        self.set_battle_speed(settings.btl_speed);
        self.stage.fixed_camera = settings.camera == 1;
        self.adapter.battle_mut().config.fixed_camera = settings.camera == 1;
    }

    /// Config `ATB: Wait` (`cfg.atb == 1`): `IsNativeEnableAtb` stops the
    /// gauges while a member's menu is past the command window. The HUD
    /// says when (`stalled`).
    pub fn set_atb_wait_stall(&mut self, stalled: bool) {
        self.adapter.battle_mut().atb_paused = stalled;
    }

    /// Caps every party member's HP at `hp` (capture tooling: a quick way
    /// to reach the game over screen).
    pub fn cap_party_hp(&mut self, hp: u32) {
        let party = self.party.len();
        let battle = self.adapter.battle_mut();
        for unit in battle.units.iter_mut().take(party) {
            unit.cur_hp = unit.cur_hp.min(hp);
        }
        for value in self.hp.iter_mut().take(party) {
            *value = (*value).min(hp);
        }
    }

    /// Turns every party member's Trance gauge on at `trance` (capture
    /// tooling: a hit or two away from Trance).
    pub fn preset_trance(&mut self, trance: u8) {
        let party = self.party.len();
        let battle = self.adapter.battle_mut();
        for unit in battle.units.iter_mut().take(party) {
            unit.has_trance_gauge = true;
            unit.trance = trance;
        }
    }

    /// Puts party member `unit` into Trance now, the way a full gauge does
    /// (`AlterStatus(Trance)` queues its `SysTrans`). Capture tooling.
    pub fn enter_trance(&mut self, unit: usize) {
        let battle = self.adapter.battle_mut();
        if let Some(member) = battle.units.get_mut(unit) {
            member.has_trance_gauge = true;
            member.trance = u8::MAX;
            battle.alter_status_on(unit, 14, &mut SplitMix64Random::new(0));
        }
    }

    /// Field entity of machine unit `unit`.
    pub fn entity(&self, unit: usize) -> Option<EntityId> {
        self.entities.get(unit).copied()
    }

    /// The party's flee roll while L1 and R1 are held
    /// (`btl_sys.CheckEscape(true)`, once a second). True when the escape
    /// was queued.
    pub fn try_flee(&mut self) -> bool {
        if self.state != FightState::Running {
            return false;
        }
        let mut services = WorldGameServices::new(&mut self.world);
        self.adapter.flee_attempt(&mut services).unwrap_or(false)
    }

    /// The command menu of party member `unit`, with its Trance commands
    /// while it is in Trance (`BattleHUD.DisplayCommand`).
    pub fn menu(&self, unit: usize) -> Option<&PartyMenu> {
        let trance =
            self.machine().units.get(unit).is_some_and(|unit| {
                unit.is_under_any_status(tantalus_legacy::battle::status::TRANCE)
            });
        if trance {
            self.trance_menu.get(unit)
        } else {
            self.menu.get(unit)
        }
    }

    /// Item `item` left in the battle inventory.
    pub fn item_count(&self, item: i32) -> u32 {
        self.machine().inventory.get(&item).copied().unwrap_or(0)
    }

    /// The record so far, for a client to draw.
    pub fn stage(&self) -> &crate::battle_stage::BattleStage {
        &self.stage
    }

    /// Names the enemies ([`crate::battle_stage::BattleStage::name_enemies`]).
    pub fn name_enemies(&mut self, text: &[String]) {
        self.stage.name_enemies(text);
    }

    pub fn ticks(&self) -> u64 {
        self.tick
    }

    pub fn is_over(&self) -> bool {
        self.state != FightState::Running
    }

    /// The outcome once the fight has stopped.
    pub fn outcome(&self) -> Option<&str> {
        self.report.outcome.as_deref()
    }

    /// The stand-in policy: the first ready party member attacks the first
    /// enemy still standing, read from the machine (units are party then
    /// enemies).
    fn stand_in(&mut self) -> Option<BattleCommand> {
        // A player who has been hit by a counter-attack (Black Waltz No. 2 answers
        // Vivi's Fire with Fira, EVT_BATTLE_AP_E012) stops casting at that enemy.
        if !self.countered {
            let machine = self.adapter.battle();
            let party_ids = (0..self.party.len()).fold(0, |ids, i| ids | machine.units[i].id);
            self.countered = machine.running.iter().chain(&machine.cmds.queue).any(|&c| {
                let slot = &machine.cmds.slots[c];
                slot.cmd_no == cmd::ENEMY_COUNTER && slot.tar_id & party_ids != 0
            });
        }
        let countered = self.countered;
        let party = &self.party;
        let gear = &self.gear;
        let ids = &self.ids;
        let entities = &self.entities;
        let machine = self.adapter.battle();
        // The first enemy still standing; once the fight has dragged on
        // (STALEMATE_TICKS) the weakest, the first on a tie: a healer beside a
        // stronger partner (the Ice Cavern pair, EVT_BATTLE_IC_E011) heals
        // the target faster than one fighter hurts it, and a player who
        // sees that picks the healer.
        let standing = |i: &usize| {
            machine.sched[*i].in_list
                && machine.units[*i].cur_hp > 0
                && !machine.units[*i].is_under_any_status(DEATH)
        };
        let mut enemies = (party.len()..machine.units.len()).filter(standing);
        let target = if self.report.ticks > STALEMATE_TICKS {
            enemies.min_by_key(|&i| machine.units[i].cur_hp)
        } else {
            enemies.next()
        };
        let actor = machine
            .ready_players
            .iter()
            .copied()
            .find(|&i| i < party.len());
        let (Some(actor), Some(target)) = (actor, target) else {
            return None;
        };
        self.sent += 1;
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
            .filter(|&i| machine.sched[i].in_list && machine.units[i].cur_hp == 0 && !healing(i))
            .min_by_key(|&i| party[i].character != 1);
        let hurt = (0..party.len())
            .filter(|&i| {
                let u = &machine.units[i];
                machine.sched[i].in_list
                    && u.cur_hp > 0
                    && u.cur_hp * 5 < u.max_hp * 3
                    && !healing(i)
            })
            .min_by_key(|&i| machine.units[i].cur_hp * 1000 / machine.units[i].max_hp.max(1));
        // Vivi keeps casting while he has the MP; the others heal.
        let caster =
            party[actor].character == 1 && has(ABILITY_FIRE) && unit.cur_mp >= 6 && !self.countered;
        let heal = match (downed, hurt) {
            _ if caster => None,
            (Some(ally), _) if count(ITEM_PHOENIX_DOWN) > 0 => Some((ids.phoenix.clone(), ally)),
            (Some(ally), _) if count(ITEM_PHOENIX_PINION) > 0 => Some((ids.pinion.clone(), ally)),
            (_, Some(ally)) if count(ITEM_POTION) > 0 => Some((ids.potion.clone(), ally)),
            (_, Some(ally)) if count(ITEM_HI_POTION) > 0 => Some((ids.hi_potion.clone(), ally)),
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
                .map(|ally| (ids.remedy.clone(), ally)),
            _ => None,
        };
        let (action, target) = if let Some(heal) = heal {
            heal
        } else {
            (
                match party[actor].character {
                    1 if has(ABILITY_FIRE) && unit.cur_mp >= 6 && !countered => ids.fire.clone(),
                    0 if has(ABILITY_FLEE)
                        && unit.is_under_any_status(tantalus_legacy::battle::status::TRANCE) =>
                    {
                        ids.free_energy.clone()
                    }
                    _ => ids.attack.clone(),
                },
                target,
            )
        };
        Some(BattleCommand {
            actor: entities[actor],
            action,
            targets: vec![entities[target]],
        })
    }

    /// One battle tick (15 a second, `BATTLE_TPS`). False once the fight
    /// has stopped (an outcome, the tick limit or an error).
    pub fn step(&mut self, control: FightControl) -> bool {
        if self.state != FightState::Running {
            return false;
        }
        if self.tick >= BATTLE_TICK_LIMIT {
            self.state = FightState::Ended;
            return false;
        }
        let tick = self.tick;
        self.tick += 1;
        match self.tick_once(tick, control) {
            Ok(true) => true,
            Ok(false) => {
                self.state = FightState::Ended;
                false
            }
            Err(error) => {
                self.state = FightState::Failed(error);
                false
            }
        }
    }

    /// The loop body of a fight; false when it ends.
    fn tick_once(&mut self, tick: u64, control: FightControl) -> Result<bool, String> {
        self.report.ticks = tick + 1;
        let mut input = CombatInput::new(Tick::new(tick));
        input.command = match control {
            FightControl::StandIn => self.stand_in(),
            FightControl::Player(command) => {
                if command.is_some() {
                    self.sent += 1;
                }
                command
            }
        };
        let mut services = WorldGameServices::new(&mut self.world);
        let advanced = self.adapter.advance(&input, &mut services);
        drop(services);
        // Script writes to units (`RunBattleCode` member sets) go back
        // into the machine, as in `battle_run`.
        let dirty = self.adapter.hooks_mut().take_dirty_views();
        apply_unit_views(self.adapter.battle_mut(), &dirty);
        let machine = self.adapter.battle();
        let phase = format!("{:?}/{:?}", machine.phase, machine.menu_off_seq);
        let (over, result) = (machine.over, machine.result);
        {
            let (max_atb, present): (Vec<i16>, Vec<bool>) = machine
                .sched
                .iter()
                .map(|sched| (sched.max_atb, sched.in_list))
                .unzip();
            self.stage
                .record(tick, &machine.units, &max_atb, &present, &phase);
            // `TANTALUS_BATTLE_LOG` set: every machine event and script
            // command on stderr, for tracing one fight.
            static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *LOG.get_or_init(|| std::env::var_os("TANTALUS_BATTLE_LOG").is_some()) {
                for (t, e) in &machine.log[self.log_seen.min(machine.log.len())..] {
                    eprintln!("battle tick {tick} t{t} {e:?}");
                }
                let events = &self.adapter.hooks().driver.state.events;
                for e in &events[self.events_seen.min(events.len())..] {
                    eprintln!("battle ai tick {tick} {e:?}");
                }
                if tick % 200 == 3 {
                    for (i, u) in machine.units.iter().enumerate() {
                        eprintln!(
                            "battle unit tick {tick} {i} lv{} hp{}/{} str{} mag{} pdef{} mdef{} status{:x}",
                            u.level,
                            u.cur_hp,
                            u.max_hp,
                            u.strength,
                            u.magic,
                            u.physical_defence,
                            u.magic_defence,
                            u.status_current
                        );
                    }
                }
            }
            let events = crate::battle_stage::stage_events(
                machine,
                &machine.log[self.log_seen.min(machine.log.len())..],
                &mut self.hp,
            );
            self.log_seen = machine.log.len();
            if let Some(frame) = self.stage.frames.last_mut() {
                frame.events = events;
            }
        }
        let report = &mut self.report;
        if report.phase_log.last().map(|entry| &entry.1) != Some(&phase) {
            let hp = machine.units[self.party.len()..]
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
                    return Ok(false);
                }
                _ => return Err(format!("{error:?}")),
            }
        }
        let party_len = self.party.len();
        for event in self.adapter.take_events() {
            if let CombatEvent::UnitDefeated { unit, .. } = event
                && let Some(enemy) = self.entities[party_len..].iter().position(|e| *e == unit)
                && !self.defeated.contains(&unit)
            {
                self.defeated.push(unit);
                if enemy < self.rows.len() {
                    self.bonus.set_bonus(&mut self.rows, enemy, &mut self.draws);
                }
            }
        }
        // Script-paid bonuses (member 71) for units that never die.
        let events = &self.adapter.hooks().driver.state.events;
        for event in &events[self.events_seen.min(events.len())..] {
            if let AiEvent::SetBonus { unit } = event
                && let Some(enemy) = self.entities[party_len..].iter().position(|e| e == unit)
                && enemy < self.rows.len()
            {
                self.bonus.set_bonus(&mut self.rows, enemy, &mut self.draws);
            }
        }
        self.events_seen = events.len();
        let report = &mut self.report;
        report.items_used = self
            .items
            .iter()
            .filter_map(|(&item, &count)| {
                let left = self
                    .adapter
                    .battle()
                    .inventory
                    .get(&item)
                    .copied()
                    .unwrap_or(0);
                let used = (count.max(0) as u32).saturating_sub(left);
                (used > 0).then_some((item, used))
            })
            .collect();
        if let Some(outcome) = self.adapter.outcome() {
            report.outcome = Some(outcome_name(outcome).to_owned());
            return Ok(false);
        }
        // The machine does not reach `over` on a defeat (the defeat song
        // fade is not ported), so the defeat phase ends the fight here.
        if self.adapter.battle().phase == Phase::Defeat {
            report.outcome = Some("defeat".to_owned());
            return Ok(false);
        }
        Ok(true)
    }

    /// Ends the fight (wherever it stopped) and fills in the report.
    pub fn finish(self, battles: &FieldBattles) -> FightReport {
        let LiveFight {
            mut report,
            party,
            adapter,
            mut stage,
            defeated,
            bonus,
            sent,
            state,
            ..
        } = self;
        if let Ok(mut seen) = battles.summons_seen.lock() {
            seen.clone_from(&adapter.battle().config.summons_seen);
        }
        report.summons_seen = Some(adapter.battle().config.summons_seen.clone());
        // `btl_sys.SavePlayerData`: a member still in Trance leaves with an
        // empty gauge.
        report.party_trance = party
            .iter()
            .zip(&adapter.battle().units)
            .map(|(fighter, unit)| {
                let trance = if unit.is_under_any_status(tantalus_legacy::battle::status::TRANCE) {
                    0
                } else {
                    unit.trance
                };
                (fighter.character, trance)
            })
            .collect();
        if let FightState::Failed(error) = state {
            report.error = Some(error);
            return report;
        }
        let equipment = &battles.equipment;
        report.items_stolen = adapter
            .battle()
            .log
            .iter()
            .filter_map(|(_, event)| match event {
                tantalus_legacy::battle::machine::BattleEvent::Stole {
                    item: Some(item), ..
                } => Some(*item),
                _ => None,
            })
            .collect();
        report.abilities_eaten = adapter
            .battle()
            .log
            .iter()
            .filter_map(|(_, event)| match event {
                tantalus_legacy::battle::machine::BattleEvent::Ate {
                    caster,
                    result: tantalus_legacy::battle::calc::EatResult::Yummy(ability),
                    ..
                } => Some((party.get(*caster)?.character, *ability)),
                _ => None,
            })
            .collect();
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
                members[slot] = Some(reward_member(fighter, unit.cur_hp, unit.cur_mp, equipment));
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
                    let (equipped, learnable) =
                        equipment.abilities(fighter.character, equipment.slots(fighter));
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
        report
    }
}

/// `btl_sys.SavePlayerData`: each party member's battle HP and MP go back
/// to the player record, clamped to the record's maximum, with its Trance
/// gauge, and the summons cast are remembered. A fight that failed to run
/// leaves the records alone.
pub fn save_party(report: &FightReport, world: &mut FieldWorld) {
    if report.error.is_some() || report.outcome.is_none() {
        return;
    }
    world.last_battle = report.battle;
    world.battle_result = battle_result(report);
    // `btl_sys.ManageBattleEnd`: a defeat in a scene without `NoGameOver` becomes
    // `BTL_RESULT_GAMEOVER` and the game over screen; the party records are not saved.
    if matches!(world.battle_result, 3 | 6) && !report.no_game_over {
        world.battle_result = 6;
        world.game_over = true;
        return;
    }
    // `TimerUI.Update` follows real time and keeps running through the battle (the Festival
    // of the Hunt clock): the fight's ticks run at `BATTLE_TPS`, the field's at 30.
    if world.timer.running {
        let field_ticks = report.ticks.saturating_mul(
            u64::from(tantalus_game::field::TimerState::TICKS_PER_SECOND as u32)
                / u64::from(tantalus_legacy::battle::atb::BATTLE_TPS),
        );
        world.timer.elapsed_ticks = world
            .timer
            .elapsed_ticks
            .saturating_add(u32::try_from(field_ticks).unwrap_or(u32::MAX));
    }
    for &(character, trance) in &report.party_trance {
        world.trance.insert(character, trance);
    }
    if let Some(seen) = &report.summons_seen {
        world.summons_seen.clone_from(seen);
    }
    let stats = &mut world.character_stats;
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
    let (equipped_abilities, learnable) =
        equipment.abilities(fighter.character, equipment.slots(fighter));
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
    for &(character, ability) in &report.abilities_eaten {
        world
            .learned_abilities
            .entry(character)
            .or_default()
            .insert(ability);
    }
    for &item in &report.items_stolen {
        if let Ok(item) = i32::try_from(item) {
            world.add_item_amount(item, 1);
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

/// `ff9.btl_result` as the field reads it after the fight (`FF9StateGlobal.BTL_RESULT_*`):
/// what a battle script's `End battle` code set, else the machine's outcome. A defeat reads 3
/// (`BTL_RESULT_DEFEAT`): `btl_sys.ManageBattleEnd` turns it into a game over unless the scene
/// has `NoGameOver` (the Festival of the Hunt), and only then does the field go on to read it.
pub fn battle_result(report: &FightReport) -> u8 {
    if let Some(code) = report.result_code {
        return code;
    }
    match report.outcome.as_deref() {
        Some("victory") => 1,
        Some("defeat") => 3,
        Some("escape") => 4,
        Some("enemy_flee") => 7,
        _ => 0,
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

/// `PLAYER.basis` dex, str, mgc, wpr after `level - 1` level-ups
/// (`ff9play.FF9Play_Build` with `lvup`, `ff9level.FF9Level_GetDex/Str/Mgc/
/// Wpr`): each level-up adds the stone bonus (str +3, mgc +3, wpr +1, as
/// `cur` is rebuilt from zero) and the equipment bonus to a `u16` counter
/// that counts a point per 32, on top of `base + level * k`. The equipment
/// bonus is the default set's, the stand-in the field side has for gear.
fn level_basis(base: &BaseStats, level: u8, equip: [i32; 4]) -> [u8; 4] {
    let lv = i32::from(level);
    let ups = lv - 1;
    let counter = |capa: i32, equip: i32| ((ups * (capa + equip)) as u16 >> 5) as i32;
    let dex = i32::from(base.speed) + lv / 10 + counter(0, equip[0]);
    let str_ = i32::from(base.strength) + lv * 3 / 10 + counter(3, equip[1]);
    let mgc = i32::from(base.magic) + lv * 3 / 10 + counter(3, equip[2]);
    let wpr = i32::from(base.spirit) + lv * 3 / 20 + counter(1, equip[3]);
    [
        dex.min(50) as u8,
        str_.min(99) as u8,
        mgc.min(99) as u8,
        wpr.min(50) as u8,
    ]
}

/// A party member at level 1: base stats, the field HP/MP and the default
/// equipment (a power-12 short-range weapon when the tables have none).
fn party_unit(slot: usize, fighter: &PartyFighter, equipment: &Equipment) -> BattleUnit {
    let slots = equipment.slots(fighter);
    let (weapon, defence) = equipment.outfit(slots);
    let mut unit = BattleUnit::new(1 << slot, true);
    unit.character_id = fighter.character as u8;
    unit.serial_no = tantalus_legacy::battle::unit::default_serial_number(
        unit.character_id,
        equipment.weapon_shape(slots),
        fighter.globals.scenario,
    )
    .unwrap_or(255);
    // `btl_init.cs:371-379`: no gauge before the scenario turns it on, for
    // category 16 (guest characters) or for Garnet while she is depressed.
    let category = tantalus_legacy::battle::unit::default_player_category(unit.character_id);
    unit.has_trance_gauge = fighter.globals.trance_gauge != 0
        && category.is_some_and(|category| category & 16 == 0)
        && !(fighter.character == 2 && fighter.globals.garnet_depress != 0);
    unit.trance = if unit.has_trance_gauge {
        fighter.trance
    } else {
        0
    };
    unit.row = fighter.row;
    unit.level = fighter.level.clamp(1, 99);
    unit.cur_hp = fighter.hp.max(1);
    unit.max_hp = fighter.max_hp.max(1);
    unit.cur_mp = fighter.mp;
    unit.max_mp = fighter.max_mp;
    // `FF9Play_Update`: `elem = basis + equipment bonus`, capped at 99.
    let bonus = equipment.stat_bonus(slots);
    let basis = level_basis(&fighter.base, unit.level, bonus);
    let stat = |base: u8, add: i32| (i32::from(base) + add).clamp(0, 99) as u8;
    unit.dexterity = stat(basis[0], bonus[0]);
    unit.strength = stat(basis[1], bonus[1]);
    unit.magic = stat(basis[2], bonus[2]);
    unit.will = stat(basis[3], bonus[3]).max(1);
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
    // `btl_eqp.InitEquipPrivilegeAttrib`.
    let [bonus_set, guard, absorb, half, weak] = equipment.element_sets(slots);
    unit.bonus_elements = Elements(bonus_set);
    unit.guard_elements = Elements(guard);
    unit.absorb_elements = Elements(absorb);
    unit.half_elements = Elements(half);
    unit.weak_elements = Elements(weak);
    apply_support_abilities(&mut unit, fighter.support);
    unit
}

/// `PLAYER.sa` into the unit: the enabled set (`ff9abil.GetEnabledSA`) and
/// the status abilities `btl_abil.CheckStatusAbility` applies through the
/// `StatusInit` features of `AbilityFeatures.txt`: Auto-Reflect/Float/
/// Haste/Regen (permanent), Auto-Life (starting status), and the resist
/// abilities (Body Temp, Insomniac, Antibody, Bright Eyes, Loudmouth,
/// Jelly, Locomotion, Clear Headed).
fn apply_support_abilities(unit: &mut BattleUnit, support: u64) {
    use tantalus_legacy::battle::status as st;
    for bit in 0..64_u8 {
        if support & (1 << bit) == 0 {
            continue;
        }
        unit.support_abilities.insert(bit);
        match bit {
            0 => unit.status_permanent |= st::REFLECT,
            1 => unit.status_permanent |= st::FLOAT,
            2 => unit.status_permanent |= st::HASTE,
            3 => unit.status_permanent |= st::REGEN,
            4 => unit.status_current |= st::AUTO_LIFE,
            40 => unit.status_resist |= st::FREEZE | st::HEAT,
            48 => unit.status_resist |= st::SLEEP,
            49 => unit.status_resist |= st::POISON | st::VENOM,
            50 => unit.status_resist |= st::BLIND,
            51 => unit.status_resist |= st::SILENCE,
            53 => unit.status_resist |= st::PETRIFY | st::GRADUAL_PETRIFY,
            57 => unit.status_resist |= st::STOP,
            58 => unit.status_resist |= st::CONFUSE,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zidane() -> PartyFighter {
        PartyFighter {
            character: 0,
            hp: 3000,
            max_hp: 3000,
            mp: 100,
            max_mp: 100,
            base: BaseStats {
                speed: 21,
                strength: 21,
                magic: 14,
                spirit: 15,
                gems: 10,
            },
            level: 1,
            globals: PartyGlobals::default(),
            row: 1,
            trance: 0,
            equip: None,
            support: 0,
        }
    }

    /// A strong party fights with its level and level-grown stats, not with
    /// level 1 ones (the Ralvuimago fight of the story probe was lost at
    /// level 80 with the level 1 strength of a new game).
    #[test]
    fn party_units_carry_the_level_and_the_grown_stats() {
        let equipment = Equipment::default();
        let fresh = party_unit(0, &zidane(), &equipment);
        assert_eq!((fresh.level, fresh.strength, fresh.dexterity), (1, 21, 21));
        let mut veteran = zidane();
        veteran.level = 99;
        let unit = party_unit(0, &veteran, &equipment);
        assert_eq!(unit.level, 99);
        // `FF9Level_GetStr`: 21 + 99 * 3 / 10 + (98 * 3 >> 5) = 59; dex 21
        // + 9 = 30; mgc 14 + 29 + 9 = 52; wpr 15 + 14 + 3 = 32.
        assert_eq!(
            (unit.strength, unit.dexterity, unit.magic, unit.will),
            (59, 30, 52, 32)
        );
    }

    #[test]
    fn stat_bonus_sums_the_bonus_rows_of_the_default_set() {
        let mut equipment = Equipment::default();
        equipment.sets.insert(0, 7);
        equipment.items.insert(7, [1, 89, -1, 100, -1]);
        equipment.bonus_ids.extend([(1, 0), (89, 2), (100, 5)]);
        equipment
            .bonuses
            .extend([(0, [0; 4]), (2, [0, 0, 0, 1]), (5, [1, 2, 0, 0])]);
        assert_eq!(
            equipment.stat_bonus(equipment.default_slots(0)),
            [1, 2, 0, 1]
        );
        assert_eq!(equipment.stat_bonus(equipment.default_slots(3)), [0; 4]);
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

    /// `TANTALUS_MEMORIA` and `TANTALUS_GAME_DATA` set: prints each effect's
    /// plugin timing next to `Sequence.seq`'s (`--nocapture`).
    #[test]
    #[ignore = "needs the game data and Memoria"]
    fn plugin_timing_next_to_the_sequences() {
        let (Some(root), Some(data)) = (
            std::env::var_os("TANTALUS_MEMORIA"),
            std::env::var_os("TANTALUS_GAME_DATA"),
        ) else {
            return;
        };
        let characters = Path::new(&root).join("Memoria.Patcher/StreamingAssets/Data/Characters");
        let mut equipment = Equipment::load(&characters).expect("Memoria tables load");
        equipment.load_plugin_timing(Path::new(&data));
        let table = &equipment.effects;
        let mut ids: Vec<(i32, bool)> = table
            .abilities
            .values()
            .flat_map(|a| [(a.0, false), (a.1, false)])
            .collect();
        ids.extend(table.items.values().map(|&id| (id, true)));
        ids.sort_unstable();
        ids.dedup();
        for (id, item) in ids {
            let start = std::time::Instant::now();
            let plugin = table.timing_of(id, item);
            let seq = table.timing.get(&id);
            println!(
                "{id} item={item}: now {:?} seq {:?} ({:?})",
                plugin.map(|t| (t.hit, t.figure, t.length)),
                seq.map(|t| (t.hit, t.figure, t.length)),
                start.elapsed()
            );
        }
    }

    /// The world's equipment (not the default set) feeds stats, defence,
    /// the weapon, element sets and enabled support abilities.
    #[test]
    fn party_unit_fights_in_the_worlds_gear() {
        let mut equipment = Equipment::default();
        equipment.sets.insert(0, 7);
        equipment.items.insert(7, [1, -1, -1, -1, -1]);
        equipment.weapons.insert(
            1,
            WeaponStats {
                power: 12,
                rate: 0,
                elements: Elements::NONE,
                category: 1,
                status: 0,
            },
        );
        equipment.weapons.insert(
            2,
            WeaponStats {
                power: 40,
                rate: 25,
                elements: Elements(1),
                category: 1,
                status: 0,
            },
        );
        equipment.weapon_status_index.insert(2, 5);
        equipment.status_sets.insert(5, 1 << 16);
        // Item 90 is armor row 2: defence 10, evade 3, magic defence 7,
        // magic evade 4, bonus 9: strength +2, guard Fire, weak Ice.
        equipment.armors.insert(2, [10, 3, 7, 4]);
        equipment.bonus_ids.insert(90, 9);
        equipment.bonuses.insert(9, [0, 2, 0, 0]);
        equipment.bonus_elements.insert(9, [0, 1, 0, 0, 2]);
        let default = party_unit(0, &zidane(), &equipment);
        assert_eq!(default.weapon.map(|w| w.power), Some(12));
        assert_eq!(default.physical_defence, 0);
        let mut fighter = zidane();
        fighter.equip = Some([2, 90, -1, -1, -1]);
        fighter.support = (1 << 4) | (1 << 49);
        let unit = party_unit(0, &fighter, &equipment);
        let weapon = unit.weapon.expect("weapon");
        assert_eq!(
            (weapon.power, weapon.elements, weapon.status),
            (40, Elements(1), 1 << 16)
        );
        assert_eq!(unit.strength, default.strength + 2);
        assert_eq!(
            (
                unit.physical_defence,
                unit.physical_evade,
                unit.magic_defence,
                unit.magic_evade
            ),
            (10, 3, 7, 4)
        );
        assert_eq!(
            (unit.guard_elements, unit.weak_elements),
            (Elements(1), Elements(2))
        );
        assert!(unit.has_support_ability(4) && unit.has_support_ability(49));
        assert_ne!(
            unit.status_current & tantalus_legacy::battle::status::AUTO_LIFE,
            0
        );
        assert_ne!(
            unit.status_resist & tantalus_legacy::battle::status::POISON,
            0
        );
    }

    #[test]
    fn loadout_reads_the_worlds_equipment_and_support_abilities() {
        let mut world = FieldWorld::new();
        assert_eq!(PartyFighter::loadout(&world, 0), (None, 0));
        world.equip(0, 0, Some(5));
        world.equip(0, 3, Some(120));
        world.support_enabled.insert(0, vec![192, 241]);
        let (equip, support) = PartyFighter::loadout(&world, 0);
        assert_eq!(equip, Some([5, -1, -1, 120, -1]));
        assert_eq!(support, 1 | 1 << 49);
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
        assert_eq!(
            equipment.stat_bonus(equipment.default_slots(11)),
            [1, 2, 3, 8]
        );
        assert_eq!(equipment.stat_bonus(equipment.default_slots(0)), [0; 4]);
        // Zidane starts with a Dagger (Flee, 40 AP to learn).
        let (equipped, learnable) = equipment.abilities(0, equipment.default_slots(0));
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
        use tantalus_game::field::world::CharacterStats;
        let mut world = FieldWorld::new();
        world.character_stats.insert(
            0,
            CharacterStats {
                current_hp: 105,
                max_hp: 105,
                current_mp: 36,
                max_mp: 36,
            },
        );
        let report = FightReport {
            outcome: Some("victory".to_owned()),
            party_hp: vec![(0, 40, 105)],
            party_mp: vec![(0, 99, 36)],
            party_trance: vec![(0, 120)],
            summons_seen: Some(vec![49]),
            ..FightReport::default()
        };
        save_party(&report, &mut world);
        assert_eq!(world.character_stats[&0].current_hp, 40);
        assert_eq!(world.character_stats[&0].current_mp, 36);
        assert_eq!(world.trance.get(&0), Some(&120));
        assert_eq!(world.summons_seen, vec![49]);

        // A fight that did not run leaves the record alone.
        let failed = FightReport {
            error: Some("no scene".to_owned()),
            party_hp: vec![(0, 1, 105)],
            ..FightReport::default()
        };
        save_party(&failed, &mut world);
        assert_eq!(world.character_stats[&0].current_hp, 40);
    }

    /// `btl_sys.ManageBattleEnd`: a lost fight is a game over unless the scene has
    /// `NoGameOver`, which goes back to the field with the defeat (`BTL_RESULT_DEFEAT`).
    #[test]
    fn a_lost_fight_is_a_game_over_unless_the_scene_allows_losing() {
        use tantalus_game::field::world::CharacterStats;
        let stats = CharacterStats {
            current_hp: 105,
            max_hp: 105,
            current_mp: 36,
            max_mp: 36,
        };
        let lost = FightReport {
            outcome: Some("defeat".to_owned()),
            party_hp: vec![(0, 0, 105)],
            ..FightReport::default()
        };
        let mut world = FieldWorld::new();
        world.character_stats.insert(0, stats);
        save_party(&lost, &mut world);
        assert!(world.game_over);
        assert_eq!(world.battle_result, 6);
        assert_eq!(world.character_stats[&0].current_hp, 105);

        let hunt = FightReport {
            no_game_over: true,
            ..lost
        };
        let mut world = FieldWorld::new();
        world.character_stats.insert(0, stats);
        save_party(&hunt, &mut world);
        assert!(!world.game_over);
        assert_eq!(world.battle_result, 3);
        assert_eq!(world.character_stats[&0].current_hp, 0);
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

/// The ability script of action `id`: the installed game's `FF9BattleDB.aa_data`
/// value where Memoria's `Actions.csv` differs and the difference is not a bug
/// fix (`audit/approved-differences.csv`, `memoria-actions-thunder-slash-script`).
/// Memoria gives Thunder Slash (149) `OriginalMagicAttackScript` (20); the
/// original runs script 17.
fn vanilla_script_id(id: i32, memoria: i32) -> i32 {
    match id {
        149 => 17,
        _ => memoria,
    }
}

#[cfg(test)]
mod vanilla_script_tests {
    use super::vanilla_script_id;

    #[test]
    fn thunder_slash_keeps_the_original_script() {
        assert_eq!(vanilla_script_id(149, 20), 17);
        assert_eq!(vanilla_script_id(150, 20), 20);
    }
}
