//! Shop bridge: run the legacy shop trade rules (`tantalus-legacy` G9/G4)
//! against field world state, so shop menus and field scripts read and write
//! the same inventory.
//!
//! The legacy [`Inventory`] is the richer model (ordered slots, key items,
//! the 99 stack cap); the field world keeps the script-facing view
//! (legacy item id -> count). [`with_shop_inventory`] is the transaction
//! boundary between them: it snapshots the world into a legacy view, runs
//! the caller's trade operation, and commits back only on success.

use super::world::{FieldWorld, MAX_GIL};
use std::collections::BTreeSet;
use tantalus_legacy::shop::{Inventory, NO_ITEM};

/// Builds a legacy inventory view of the field world's regular-item map.
///
/// Keys that are not regular legacy item ids (negative, above `u16`, or the
/// `NoItem` sentinel 255) are not shop-managed and stay untouched by
/// [`apply_inventory_to_world`]. Counts clamp to the 99 stack cap.
pub fn inventory_from_world(world: &FieldWorld) -> Inventory {
    let mut inventory = Inventory::default();
    for (&id, &count) in &world.items {
        if count <= 0 {
            continue;
        }
        let Ok(id16) = u16::try_from(id) else {
            continue;
        };
        if id16 == NO_ITEM {
            continue;
        }
        inventory.slots.push(tantalus_legacy::shop::Slot {
            id: id16,
            count: count.clamp(0, 99) as u8,
        });
    }
    inventory
}

/// Syncs a (possibly mutated) legacy inventory back into the field world's
/// item map. Slots emptied by a sale disappear from the map; non-regular
/// world keys survive untouched.
pub fn apply_inventory_to_world(inventory: &Inventory, world: &mut FieldWorld) {
    let mut live = BTreeSet::new();
    for slot in &inventory.slots {
        if slot.count == 0 || slot.id == NO_ITEM {
            continue;
        }
        let id = i32::from(slot.id);
        live.insert(id);
        world.items.insert(id, i32::from(slot.count));
    }
    world.items.retain(|id, count| {
        if *count <= 0 {
            return false;
        }
        match u16::try_from(*id) {
            Ok(id16) if id16 != NO_ITEM => live.contains(id),
            _ => true,
        }
    });
}

/// Runs `trade` against a legacy inventory + gil view of the world and
/// commits the result back. The closure receives the mutable view; when it
/// returns `Ok`, the world's item map and gil are updated from the view
/// (gil clamped to [`MAX_GIL`], the reference `9_999_999` cap). On `Err`
/// the world is left exactly as it was.
///
/// Key-item obtained/used sets are not modeled in field state; callers that
/// trade key items keep those sets outside this bridge (the regular-item
/// slots still round trip).
pub fn with_shop_inventory<T, E>(
    world: &mut FieldWorld,
    trade: impl FnOnce(&mut Inventory, &mut u32) -> Result<T, E>,
) -> Result<T, E> {
    let mut inventory = inventory_from_world(world);
    let mut gil = world.gil;
    let result = trade(&mut inventory, &mut gil)?;
    world.gil = gil.min(MAX_GIL);
    apply_inventory_to_world(&inventory, world);
    Ok(result)
}
