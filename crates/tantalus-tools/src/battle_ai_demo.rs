//! `tantalusctl assets inspect-bundle --path p0data7.bin --battle-ai-demo <scene> <out json>`:
//! loads one `EVT_BATTLE_*` script from `p0data7.bin` and its scene from the
//! sibling `p0data2.bin`, runs the enemy ATB function (tag 5) of every enemy
//! through the battle script host with a fixed random sequence and writes the
//! chosen attack indices.
//!
//! The party and enemy stats are stand-ins (four full-health party members,
//! enemy HP and stats from the scene's first pattern), so the picks show what
//! the script does with that state and that draw sequence. They are not a
//! recording of a real battle and make no gameplay claim.

use std::collections::BTreeMap;
use std::path::Path;
use tantalus_assets::pc::battle_scene::{BattleBundleData, BattleScene, ByteOrTokens};
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::text_asset::TextAsset;
use tantalus_assets::pc::unity_raw::UnityRawBundle;
use tantalus_contracts::EntityId;
use tantalus_legacy::battle::host::{BattleDataView, UnitView};
use tantalus_legacy::battle::rng::BattleRandom;
use tantalus_legacy::battle::script_host::{
    AiEvent, BattleScriptDriver, BattleScriptState, ScriptTable,
};
use tantalus_legacy::{EventBinary, ExpressionOp, InstructionKind};

/// The fixed random sequence of the demo: `Comn.random8` draws, repeated.
pub const DEMO_DRAWS: [u8; 8] = [17, 203, 64, 141, 250, 3, 99, 180];

/// Rounds of "ATB full" asked of each enemy.
pub const DEMO_ROUNDS: usize = 6;

/// Party character ids of the stand-in party: Zidane, Vivi, Garnet, Steiner.
pub const DEMO_PARTY: [u8; 4] = [0, 1, 2, 3];

/// A repeating `random8` list; the other draw kinds are derived from it.
pub struct CycleRandom {
    values: Vec<u8>,
    next: usize,
    /// Draws taken so far.
    pub taken: usize,
}

impl CycleRandom {
    pub fn new(values: &[u8]) -> Self {
        Self {
            values: values.to_vec(),
            next: 0,
            taken: 0,
        }
    }

    fn draw(&mut self) -> i32 {
        let value = self.values[self.next % self.values.len()];
        self.next += 1;
        self.taken += 1;
        i32::from(value)
    }
}

impl BattleRandom for CycleRandom {
    fn random16(&mut self) -> i32 {
        self.draw() * 257
    }

    fn random8(&mut self) -> i32 {
        self.draw()
    }

    fn random_range(&mut self, min: i32, max_exclusive: i32) -> i32 {
        min + self.draw() % (max_exclusive - min).max(1)
    }

    fn random_unit(&mut self) -> f32 {
        self.draw() as f32 / 256.0
    }
}

/// Every `EVT_BATTLE_*` script in the bundle, by upper-case name without the
/// `.eb` suffix.
pub fn load_battle_scripts(bundle_path: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let bundle = UnityRawBundle::open(bundle_path).map_err(|error| error.to_string())?;
    let bytes = bundle
        .read_entry(0, 300 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    let serialized = SerializedFile::parse(&bytes, &bundle_path.to_string_lossy())
        .map_err(|error| error.to_string())?;
    let mut scripts = BTreeMap::new();
    for (index, object) in serialized.report().objects.iter().enumerate() {
        if object.class_id != 49 {
            continue;
        }
        let asset = TextAsset::parse(
            serialized
                .object_data(index)
                .map_err(|error| error.to_string())?,
            "battle-script",
        )
        .map_err(|error| error.to_string())?;
        if !asset.name.starts_with("EVT_BATTLE_") {
            continue;
        }
        let name = asset
            .name
            .trim_end_matches(".eb")
            .trim_end_matches(".EB")
            .to_ascii_uppercase();
        scripts.entry(name).or_insert(asset.script);
    }
    Ok(scripts)
}

/// True when some entry's ATB function (tag 5) both draws a random number
/// (system variable 0 or a random member pick) and queues an enemy attack.
pub fn uses_random_and_queues_attack(table: &ScriptTable) -> bool {
    table.entries.iter().any(|entry| {
        entry.functions.get(&5).is_some_and(|program| {
            let mut random = false;
            let mut attack = false;
            for instruction in program.instructions.values() {
                match &instruction.kind {
                    InstructionKind::QueueEnemyAttack { attack: expression } => {
                        attack = true;
                        random |= expression.0.iter().any(is_random_op);
                    }
                    InstructionKind::Evaluate(expression) => {
                        random |= expression.0.iter().any(is_random_op);
                    }
                    _ => {}
                }
            }
            random && attack
        })
    })
}

fn is_random_op(op: &ExpressionOp) -> bool {
    matches!(
        op,
        ExpressionOp::SystemVariable(0) | ExpressionOp::SelectRandomMember
    )
}

/// The first script (by name) that [`uses_random_and_queues_attack`].
pub fn first_random_attack_scene(scripts: &BTreeMap<String, Vec<u8>>) -> Option<String> {
    scripts.iter().find_map(|(name, bytes)| {
        let binary = EventBinary::parse(bytes).ok()?;
        uses_random_and_queues_attack(&ScriptTable::from_binary(&binary)).then(|| name.clone())
    })
}

fn category_class(set: &ByteOrTokens) -> u8 {
    match set {
        ByteOrTokens::Byte(value) => *value,
        ByteOrTokens::Tokens(_) => 0,
    }
}

fn element_byte(set: &ByteOrTokens) -> u8 {
    match set {
        ByteOrTokens::Byte(value) => *value,
        ByteOrTokens::Tokens(_) => 0,
    }
}

/// Stand-in battle state for `scene`'s first pattern: four full-health party
/// members (ids 1, 2, 4, 8) and one unit per placement (ids 16, 32, ...).
pub fn demo_state(scene: &BattleScene) -> Result<(BattleScriptState, Vec<u16>), String> {
    let pattern = scene
        .patterns
        .first()
        .ok_or_else(|| "scene has no pattern".to_owned())?;
    let mut units = BTreeMap::new();
    for slot in 0..4u64 {
        units.insert(
            EntityId::new(1 << slot),
            UnitView {
                max_hp: 500,
                hp: 500,
                max_mp: 100,
                mp: 100,
                level: 20,
                is_player: true,
                row: 1,
                strength: 30,
                magic: 30,
                targetable: 1,
                ..UnitView::default()
            },
        );
    }
    let mut enemy_ids = Vec::new();
    for (k, placement) in pattern
        .monsters
        .iter()
        .take(usize::from(pattern.monster_count))
        .enumerate()
    {
        let monster = scene
            .monster_types
            .get(usize::from(placement.type_no))
            .ok_or_else(|| format!("monster type {} missing", placement.type_no))?;
        let id = 16u16 << k;
        enemy_ids.push(id);
        units.insert(
            EntityId::new(u64::from(id)),
            UnitView {
                max_hp: monster.max_hp as i32,
                hp: monster.max_hp as i32,
                max_mp: monster.max_mp as i32,
                mp: monster.max_mp as i32,
                level: i32::from(monster.level),
                status_invalid: monster.resist_status,
                status_permanent: monster.auto_status,
                element_invalid: element_byte(&monster.guard_element),
                element_absorb: element_byte(&monster.absorb_element),
                element_half: element_byte(&monster.half_element),
                element_weak: element_byte(&monster.weak_element),
                targetable: i32::from(placement.flags & 1),
                class: category_class(&monster.category),
                strength: i32::from(monster.stats.strength),
                magic: i32::from(monster.stats.magic),
                physical_defence: monster.physical_defence,
                physical_evade: monster.physical_evade,
                magical_defence: monster.magical_defence,
                magical_evade: monster.magical_evade,
                ..UnitView::default()
            },
        );
    }
    let enemy_mask = enemy_ids
        .iter()
        .fold(0u32, |mask, id| mask | u32::from(*id));
    let data = BattleDataView {
        battle_map_no: i32::from(scene.header.battle_map_index.unwrap_or(0)),
        pattern_id: 0,
        btl_phase: 4,
        player_id_mask: 0x0F,
        enemy_id_mask: enemy_mask,
        all_id_mask: 0x0F | enemy_mask,
        ..BattleDataView::default()
    };
    Ok((BattleScriptState::new(data, units), enemy_ids))
}

/// Runs the demo and returns the JSON report.
pub fn run_demo(
    scripts_path: &Path,
    scene_name: &str,
    out: &Path,
    json: bool,
) -> Result<(), String> {
    let scripts = load_battle_scripts(scripts_path)?;
    let name = if scene_name.eq_ignore_ascii_case("first-random") {
        first_random_attack_scene(&scripts)
            .ok_or_else(|| "no battle script uses RANDOM with a queued attack".to_owned())?
    } else {
        let upper = scene_name.to_ascii_uppercase();
        if upper.starts_with("EVT_BATTLE_") {
            upper
        } else {
            format!("EVT_BATTLE_{upper}")
        }
    };
    let bytes = scripts
        .get(&name)
        .ok_or_else(|| format!("battle script `{name}` was not found"))?;
    let scenes_path = scripts_path
        .parent()
        .map(|dir| dir.join("p0data2.bin"))
        .ok_or_else(|| "bundle path has no directory".to_owned())?;
    let data = BattleBundleData::load(&scenes_path).map_err(|error| error.to_string())?;
    let (_, scene) = data
        .scene(&name)
        .ok_or_else(|| format!("battle scene `{name}` was not found in p0data2.bin"))?;
    let scene = scene.as_ref().map_err(|error| error.to_string())?;
    let report = demo_report(&name, bytes, scene)?;
    let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(out, format!("{text}\n")).map_err(|error| error.to_string())?;
    if json {
        println!("{text}");
    } else {
        println!("{name}: AI demo written to {}", out.display());
    }
    Ok(())
}

/// The demo document for one script and scene.
pub fn demo_report(
    name: &str,
    script: &[u8],
    scene: &BattleScene,
) -> Result<serde_json::Value, String> {
    let binary = EventBinary::parse(script).map_err(|error| error.to_string())?;
    let table = ScriptTable::from_binary(&binary);
    let uses_random = uses_random_and_queues_attack(&table);
    let (state, enemy_ids) = demo_state(scene)?;
    let rng = Box::new(CycleRandom::new(&DEMO_DRAWS));
    let mut driver = BattleScriptDriver::new(table, state, rng, &DEMO_PARTY);
    for _ in 0..8 {
        driver.tick().map_err(|error| error.to_string())?;
    }
    let mut enemies = Vec::new();
    for id in &enemy_ids {
        let slot = id.trailing_zeros() as usize;
        let entry = driver.object_entry(slot);
        let mut rounds = Vec::new();
        for round in 0..DEMO_ROUNDS {
            let accepted = driver.request_atb(*id);
            let picked = if accepted {
                driver
                    .run_until_attack(*id, 256)
                    .map_err(|error| error.to_string())?
            } else {
                None
            };
            let attack_row = picked
                .and_then(|command| usize::try_from(command.attack).ok())
                .and_then(|index| scene.attacks.get(index))
                .map(|attack| {
                    serde_json::json!({
                        "script_id": attack.script_id,
                        "power": attack.power,
                        "rate": attack.rate,
                        "target": attack.target,
                        "mp": attack.mp,
                    })
                });
            rounds.push(serde_json::json!({
                "round": round,
                "request_accepted": accepted,
                "attack_index": picked.map(|c| c.attack),
                "target_mask": picked.map(|c| c.target_mask),
                "scene_attack": attack_row,
            }));
        }
        enemies.push(serde_json::json!({
            "btl_id": id,
            "object_slot": slot,
            "script_entry": entry,
            "rounds": rounds,
        }));
    }
    let other_events: Vec<String> = driver
        .state
        .events
        .iter()
        .filter(|event| !matches!(event, AiEvent::Command(_)))
        .map(|event| format!("{event:?}"))
        .collect();
    Ok(serde_json::json!({
        "scope": "enemy ATB function (tag 5) run through the battle script host with stand-in party and enemy stats and a fixed random8 sequence; not a recording of a real battle and not a gameplay claim",
        "scene": name,
        "uses_random_and_queues_attack": uses_random,
        "random8_sequence": DEMO_DRAWS,
        "party_character_ids": DEMO_PARTY,
        "pattern": 0,
        "enemies": enemies,
        "other_events": other_events,
    }))
}
