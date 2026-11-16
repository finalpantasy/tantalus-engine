//! Battle scene -> script reward rows: map decoded `SB2_*` monster data onto
//! the legacy reward shape so the reward path (`SetBonus` draws, EXP/AP/gil
//! split, field commit) runs on real scenes, not just hand-built rows.
//!
//! Sources: `tantalus_assets::pc::battle_scene` (the pinned `SB2_MON_PARM`
//! / `SB2_PATTERN` layout) and `tantalus_legacy::battle::rewards`
//! (`btl_sys.SetBonus`, `btl_init.cs:167-175`).

use tantalus_assets::pc::battle_scene::{BattleScene, ByteOrTokens};
use tantalus_legacy::battle::rewards::{BattleBonus, EnemyRewardData, NO_CARD};
use tantalus_legacy::battle::{BattleUnit, Elements};

/// Builds the per-enemy reward rows for one battle pattern: one row per
/// placement, in pattern order. `type_id` is the monster type index — the
/// reference compares `et` (enemy-type) pointers, and the type index is what
/// identifies same-type enemies for the rare-slot blanking rule.
///
/// Empty item slots keep the scene's raw ids; vanilla scenes mark empties
/// with the `NoItem` sentinel (255), which `SetBonus` already skips. Returns
/// `None` when the pattern or a referenced monster type is missing.
pub fn pattern_rewards(scene: &BattleScene, pattern: usize) -> Option<Vec<EnemyRewardData>> {
    let pattern = scene.patterns.get(pattern)?;
    let mut rows = Vec::new();
    for placement in pattern
        .monsters
        .iter()
        .take(usize::from(pattern.monster_count))
    {
        let monster = scene.monster_types.get(usize::from(placement.type_no))?;
        rows.push(EnemyRewardData {
            type_id: u32::from(placement.type_no),
            present: true,
            gil: monster.win_gil,
            exp: monster.win_exp,
            items: monster.win_items,
            item_rates: monster.win_item_rates,
            card: u32::try_from(monster.win_card).unwrap_or(NO_CARD),
            card_rate: monster.win_card_rate,
            bonus_given: false,
        });
    }
    Some(rows)
}

/// The zeroed `BattleBonus` a pattern pays into: pattern AP, the
/// participation mask, and the after-event flag. Enemy deaths add gil/EXP
/// and draw items through [`tantalus_legacy::battle::rewards::BattleBonus::set_bonus`].
pub fn pattern_bonus(
    scene: &BattleScene,
    pattern: usize,
    member_flag: u8,
    event: bool,
) -> Option<BattleBonus> {
    let row = scene.patterns.get(pattern)?;
    // Rows are validated here too so a bonus never exists for a broken scene.
    pattern_rewards(scene, pattern)?;
    Some(BattleBonus::new(row.ap, member_flag, event))
}

/// Sanity check that the rows pay the same totals the scene declares.
pub fn reward_totals(rows: &[EnemyRewardData]) -> (u32, u32) {
    (
        rows.iter()
            .filter(|row| row.present)
            .map(|row| row.gil)
            .sum(),
        rows.iter()
            .filter(|row| row.present)
            .map(|row| row.exp)
            .sum(),
    )
}

/// Element/category byte under the established convention
/// (`battle_ai_demo::element_byte`): raw bytes carry through, token lists
/// read as empty until version-10 token semantics land.
fn set_byte(set: &ByteOrTokens) -> u8 {
    match set {
        ByteOrTokens::Byte(value) => *value,
        ByteOrTokens::Tokens(_) => 0,
    }
}

/// Builds machine battle units for one pattern: one enemy per used placement
/// (`btl_id` bits 16, 32, ...), with stats, defenses, elements, and status
/// masks from the monster type. Spirit and speed feed `will`/`dexterity` so
/// the attack calculator's `Target.Will` division and the ATB gauge have
/// real values; undead/flight derive from the category byte (`Undead = 16`,
/// `Flight = 128`, an inference from the `EnemyCategory` bits — token lists
/// read as neither). Party units are caller-built and prepended.
pub fn pattern_machine_units(scene: &BattleScene, pattern: usize) -> Option<Vec<BattleUnit>> {
    let pattern = scene.patterns.get(pattern)?;
    let mut units = Vec::new();
    for (k, placement) in pattern
        .monsters
        .iter()
        .take(usize::from(pattern.monster_count))
        .enumerate()
    {
        let monster = scene.monster_types.get(usize::from(placement.type_no))?;
        let mut unit = BattleUnit::new(16u16 << k, false);
        unit.level = monster.level;
        unit.cur_hp = monster.max_hp;
        unit.max_hp = monster.max_hp;
        unit.cur_mp = monster.max_mp;
        unit.max_mp = monster.max_mp;
        unit.strength = monster.stats.strength;
        unit.magic = monster.stats.magic;
        unit.dexterity = monster.stats.speed;
        unit.will = monster.stats.spirit;
        unit.physical_defence = monster.physical_defence;
        unit.physical_evade = monster.physical_evade;
        unit.magic_defence = monster.magical_defence;
        unit.magic_evade = monster.magical_evade;
        unit.guard_elements = Elements(set_byte(&monster.guard_element));
        unit.half_elements = Elements(set_byte(&monster.half_element));
        unit.absorb_elements = Elements(set_byte(&monster.absorb_element));
        unit.weak_elements = Elements(set_byte(&monster.weak_element));
        unit.status_resist = monster.resist_status;
        unit.status_permanent = monster.auto_status;
        unit.status_current = monster.initial_status;
        let category = set_byte(&monster.category);
        unit.is_undead = category & 16 != 0;
        unit.is_flight = category & 128 != 0;
        units.push(unit);
    }
    Some(units)
}

/// One-call battle assembly from decoded scene data: machine units for the
/// pattern's enemies appended to the caller's party, an adapter with entity
/// mappings wired, and the reward rows/AP the death-driven reward loop pays.
/// `players` are `(field entity, machine unit)` in party order; enemy
/// entities are `EntityId::new(btl_id)` and returned for target maps and
/// death events.
pub struct BattleSetup {
    pub adapter: tantalus_game::battle::legacy::LegacyBattleAdapter,
    pub reward_rows: Vec<EnemyRewardData>,
    pub pattern_ap: u32,
    pub enemy_entities: Vec<tantalus_contracts::EntityId>,
}

pub fn assemble_legacy_battle(
    scene: &BattleScene,
    pattern: usize,
    config: &tantalus_legacy::battle::machine::BattleConfig,
    table: &tantalus_legacy::battle::StatusTable,
    players: &[(tantalus_contracts::EntityId, BattleUnit)],
    action_map: std::collections::BTreeMap<tantalus_contracts::ContentId, (u32, i32)>,
) -> Option<BattleSetup> {
    let row = scene.patterns.get(pattern)?;
    let mut units: Vec<BattleUnit> = players.iter().map(|(_, unit)| unit.clone()).collect();
    let mut entity_pairs: Vec<(tantalus_contracts::EntityId, u16)> = players
        .iter()
        .map(|(entity, unit)| (*entity, unit.id))
        .collect();
    for unit in pattern_machine_units(scene, pattern)? {
        entity_pairs.push((
            tantalus_contracts::EntityId::new(u64::from(unit.id)),
            unit.id,
        ));
        units.push(unit);
    }
    let (unit_map, target_bits) = tantalus_game::battle::legacy::unit_mappings(&entity_pairs);
    let adapter = tantalus_game::battle::legacy::LegacyBattleAdapter::new(
        tantalus_legacy::battle::machine::Battle::new(units, config.clone(), table.clone()),
        action_map,
        unit_map,
        target_bits,
    );
    Some(BattleSetup {
        adapter,
        reward_rows: pattern_rewards(scene, pattern)?,
        pattern_ap: row.ap,
        enemy_entities: entity_pairs[players.len()..]
            .iter()
            .map(|(entity, _)| *entity)
            .collect(),
    })
}
