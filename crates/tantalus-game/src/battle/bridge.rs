//! Bridge from the legacy battle result model (B8, `tantalus-legacy`) to the
//! field commit path (`RewardBundle`).
//!
//! The legacy side computes a [`BattleReward`] from the global bonus
//! (`tantalus-legacy/src/battle/rewards.rs`, pinned `BattleResultUI`
//! semantics); this module converts it into the bundle the encounter
//! coordinator commits through [`crate::battle::GameServices`], so a finished
//! battle pays the field world with no per-engine adaptation.

use super::RewardBundle;
use tantalus_contracts::ContentId;
use tantalus_legacy::battle::rewards::BattleReward;

/// Converts a computed legacy battle reward into the field commit bundle.
///
/// * `lines` maps the four battle party lines to character ids in the
///   caller's party order; `None` skips the line. Line-to-character mapping
///   is caller knowledge (the battle engine arranges the party).
/// * Experience shares become the bundle's `shares` (the reference split has
///   already happened; the bundle total is set to the awarded sum, since the
///   remainder is lost by design). An all-empty share list stays `Some`, so a
///   battle that paid nobody suppresses the full-grant fallback.
/// * Item stacks expand to one content identity per unit; legacy ids become
///   `ffix:item/<id>`, matching [`crate::field::world::default_item_id_map`].
/// * `card` is dropped: the Tetra Master collection is not modeled in field
///   world state.
/// * `resolution_id` feeds the field's exactly-once ledger; the caller
///   allocates it once per battle outcome.
///
/// The conversion borrows: the legacy apply-once flag protects the
/// `PartySnapshot`, while the field side deduplicates by `resolution_id`, so
/// converting twice with the same id cannot double-pay.
pub fn bundle_from_battle_reward(
    reward: &BattleReward,
    lines: [Option<i32>; 4],
    resolution_id: u64,
) -> RewardBundle {
    let shares = Some(
        reward
            .shares
            .iter()
            .zip(lines)
            .filter_map(|(share, character)| {
                let character = character?;
                share.exp.map(|exp| (character, u64::from(exp)))
            })
            .collect(),
    );
    let experience = shares
        .as_ref()
        .map(|shares: &std::collections::BTreeMap<i32, u64>| shares.values().sum())
        .unwrap_or(0);
    let items = reward
        .items
        .iter()
        .flat_map(|&(legacy_id, count)| {
            let id = ContentId::parse(&format!("ffix:item/{legacy_id}"))
                .expect("numeric legacy item id is a valid content identity");
            std::iter::repeat_n(id, usize::try_from(count).unwrap_or(usize::MAX))
        })
        .collect();
    RewardBundle {
        resolution_id,
        experience,
        gil: u64::from(reward.gil),
        items,
        shares,
    }
}
