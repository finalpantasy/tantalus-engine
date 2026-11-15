//! Field item use bridge: using an item from the field menu on a party
//! member runs the legacy field calculator (`tantalus-legacy` G4 item
//! effects) against field world state.
//!
//! The legacy calculator ([`use_item`]) reads and writes a
//! [`FieldTarget`] (hp/mp/status) and consumes from an [`Inventory`]. The
//! field world holds hp/mp in [`CharacterStats`] and items in the
//! script-facing map, so this bridge adapts both sides and reuses the shop
//! transaction for the inventory. Status bits are not modeled in field
//! state; targets start with no status (the reference applies item effects
//! to `PLAYER.status`, which the field world does not track yet).

use super::shop::with_shop_inventory;
use super::world::{CharacterStats, FieldWorld};
use tantalus_legacy::shop::{
    FieldEffect, FieldItemError, FieldTarget, ItemDb, ItemUseOutcome, use_item,
};

/// Failure of [`use_field_item`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldItemUseError {
    /// The item or branch is not usable (see [`FieldItemError`]).
    Field(FieldItemError),
    /// The character has no stats in world state (not in the party table).
    UnknownCharacter,
}

impl From<FieldItemError> for FieldItemUseError {
    fn from(error: FieldItemError) -> Self {
        Self::Field(error)
    }
}

/// Uses `item` once from the field menu on `character` (`ItemUI.cs:317-450`
/// target-confirm path). On `Used` the item is consumed from the world
/// inventory and the character's hp/mp are updated; on `NoEffect` nothing
/// changes; on `Err` nothing changes. The status half of effects is inert
/// (field state tracks no status bits).
pub fn use_field_item(
    world: &mut FieldWorld,
    db: &ItemDb,
    item: u16,
    effect: &FieldEffect,
    character: i32,
    random8: impl FnOnce() -> u32,
) -> Result<ItemUseOutcome, FieldItemUseError> {
    let Some(stats) = world.character_stats.get(&character).copied() else {
        return Err(FieldItemUseError::UnknownCharacter);
    };
    let mut target = target_from_stats(&stats);
    let outcome = with_shop_inventory(world, |inventory, _gil| {
        use_item(db, inventory, item, effect, &mut target, random8)
    })
    .map_err(FieldItemUseError::Field)?;
    if let ItemUseOutcome::Used { .. } = outcome {
        let stats = world
            .character_stats
            .get_mut(&character)
            .expect("character stats checked above");
        stats.current_hp = target.hp as i32;
        stats.current_mp = target.mp as i32;
    }
    Ok(outcome)
}

fn target_from_stats(stats: &CharacterStats) -> FieldTarget {
    FieldTarget {
        hp: u32::try_from(stats.current_hp).unwrap_or(0),
        max_hp: u32::try_from(stats.max_hp).unwrap_or(0),
        mp: u32::try_from(stats.current_mp).unwrap_or(0),
        max_mp: u32::try_from(stats.max_mp).unwrap_or(0),
        status: 0,
        permanent_status: 0,
    }
}
