//! Battle scene -> script reward rows: map decoded `SB2_*` monster data onto
//! the legacy reward shape so the reward path (`SetBonus` draws, EXP/AP/gil
//! split, field commit) runs on real scenes, not just hand-built rows.
//!
//! Sources: `tantalus_assets::pc::battle_scene` (the pinned `SB2_MON_PARM`
//! / `SB2_PATTERN` layout) and `tantalus_legacy::battle::rewards`
//! (`btl_sys.SetBonus`, `btl_init.cs:167-175`).

use tantalus_assets::pc::battle_scene::BattleScene;
use tantalus_legacy::battle::rewards::{BattleBonus, EnemyRewardData, NO_CARD};

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
