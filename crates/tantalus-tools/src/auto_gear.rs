//! The trace auto-player's gear choices (a stand-in for a player, not game
//! behavior): wear the best owned weapon and armor, and buy better pieces at
//! the shops the route passes (`ShopItems.csv`). "Best" is the stand-in's
//! own measure, [`score`]: weapon power, or the armor's defences and
//! evades with the stat bonuses; accessories are left alone (their worth is
//! the support abilities). Equipping follows `EquipUI` (the old piece goes
//! back to the bag, the new one leaves it) and buying follows the shop's
//! price (`FF9ITEM_DATA.price`).

use tantalus_game::FieldWorld;
use tantalus_legacy::shop::{ItemDb, can_equip, equip_mask_from_char_id};

/// Slots the stand-in changes: weapon, head, wrist, armor (not the
/// accessory, slot 4).
const SLOTS: [usize; 4] = [0, 1, 2, 3];

/// The stand-in's worth of `item` in `slot`: weapon power (attack is what
/// wins its fights), else physical and magic defence and evade; the stat
/// bonuses (`Stats.csv`) count on both. Ties go to the dearer item.
pub fn score(db: &ItemDb, item: u16, slot: usize) -> (i32, u32) {
    let Some(data) = db.get(item) else {
        return (i32::MIN, 0);
    };
    let bonus = db
        .bonuses
        .get(data.bonus)
        .map_or(0, |b| b.dex + b.str_ + b.mgc + b.wpr);
    let base = if slot == 0 {
        data.weapon_power.unwrap_or(0) * 4
    } else {
        data.armor
            .map_or(0, |a| 2 * a.p_def + 2 * a.m_def + a.p_ev + a.m_ev)
    };
    (base + bonus, data.price)
}

/// The members whose gear the stand-in sees to: the party, then the rest of
/// the roster (`party_reserve`), in id order.
pub fn members(world: &FieldWorld) -> Vec<i32> {
    let mut members: Vec<i32> = world.party.clone();
    let reserve = world.party_reserve.unwrap_or(0);
    for character in 0..12 {
        if reserve >> character & 1 != 0 && !members.contains(&character) {
            members.push(character);
        }
    }
    members
}

/// Puts on each member's best owned piece per slot ([`score`]), party
/// first. Returns the changes as `(character, slot, item)`.
pub fn equip_best(world: &mut FieldWorld, db: &ItemDb, members: &[i32]) -> Vec<(i32, usize, u16)> {
    let mut changes = Vec::new();
    for &member in members {
        let mask = equip_mask_from_char_id(member as u32);
        for slot in SLOTS {
            let current = world.equipped(member)[slot];
            let best = world
                .items
                .iter()
                .filter(|&(_, &count)| count > 0)
                .filter_map(|(&item, _)| u16::try_from(item).ok())
                .filter(|&item| can_equip(db, item, mask, slot))
                .max_by_key(|&item| (score(db, item, slot), std::cmp::Reverse(item)));
            let Some(best) = best else { continue };
            let better =
                current.is_none_or(|current| score(db, best, slot) > score(db, current, slot));
            if !better {
                continue;
            }
            world.add_item_amount(i32::from(best), -1);
            if world
                .items
                .get(&i32::from(best))
                .is_some_and(|&count| count <= 0)
            {
                world.items.remove(&i32::from(best));
            }
            if let Some(old) = current {
                world.add_item_amount(i32::from(old), 1);
            }
            world.equip(member, slot, Some(best));
            changes.push((member, slot, best));
        }
    }
    changes
}

/// Buys, for each member in turn, the best piece of `stock` per slot that
/// beats what the member wears and what the bag holds for it, while the gil
/// lasts; then wears it ([`equip_best`]). Returns the items bought.
pub fn buy_best(world: &mut FieldWorld, db: &ItemDb, stock: &[u16], members: &[i32]) -> Vec<u16> {
    let mut bought = Vec::new();
    for &member in members {
        let mask = equip_mask_from_char_id(member as u32);
        for slot in SLOTS {
            let owned = world
                .items
                .iter()
                .filter(|&(_, &count)| count > 0)
                .filter_map(|(&item, _)| u16::try_from(item).ok())
                .chain(world.equipped(member)[slot])
                .filter(|&item| can_equip(db, item, mask, slot))
                .map(|item| score(db, item, slot))
                .max();
            let pick = stock
                .iter()
                .copied()
                .filter(|&item| can_equip(db, item, mask, slot))
                .filter(|&item| db.get(item).is_some_and(|data| data.price <= world.gil))
                .filter(|&item| owned.is_none_or(|owned| score(db, item, slot).0 > owned.0))
                .max_by_key(|&item| score(db, item, slot));
            if let Some(item) = pick {
                let price = db.get(item).map_or(0, |data| data.price);
                world.gil -= price;
                world.add_item_amount(i32::from(item), 1);
                bought.push(item);
                equip_best(world, db, &[member]);
            }
        }
    }
    bought
}

/// The medicine the stand-in keeps, `(item, count)`, bought first to last:
/// Phoenix Down, Remedy, Hi-Potion, Potion, Soft, Antidote, Eye Drops and
/// Echo Screen.
pub const SUPPLIES: [(u16, i32); 8] = [
    (240, 20),
    (247, 15),
    (237, 30),
    (236, 30),
    (242, 8),
    (243, 5),
    (244, 5),
    (241, 5),
];

/// Buys the [`SUPPLIES`] `stock` sells, one of each in turn, up to each
/// count while the gil lasts (a player stocking up before the next
/// dungeon). Returns the items bought.
pub fn buy_supplies(world: &mut FieldWorld, db: &ItemDb, stock: &[u16]) -> Vec<u16> {
    let mut bought = Vec::new();
    loop {
        let mut any = false;
        for (item, target) in SUPPLIES {
            let Some(price) = db.get(item).map(|data| data.price) else {
                continue;
            };
            let have = world.items.get(&i32::from(item)).copied().unwrap_or(0);
            if stock.contains(&item) && have < target && price > 0 && price <= world.gil {
                world.gil -= price;
                world.add_item_amount(i32::from(item), 1);
                bought.push(item);
                any = true;
            }
        }
        if !any {
            return bought;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_legacy::shop::data::item_type;
    use tantalus_legacy::shop::{ArmorData, ItemData};

    fn item(kind: u8, price: u32, power: Option<i32>, def: i32) -> ItemData {
        ItemData {
            price,
            selling_price: 0,
            equip: u64::MAX,
            kind,
            sort: 0.0,
            eq_lv: 0.0,
            bonus: 0,
            ability: Vec::new(),
            weapon_power: power,
            armor: (power.is_none()).then_some(ArmorData {
                p_def: def,
                p_ev: 0,
                m_def: 0,
                m_ev: 0,
            }),
        }
    }

    fn db() -> ItemDb {
        let mut db = ItemDb::default();
        db.items
            .insert(1, item(item_type::WEAPON, 100, Some(12), 0));
        db.items
            .insert(2, item(item_type::WEAPON, 300, Some(16), 0));
        db.items
            .insert(3, item(item_type::WEAPON, 900, Some(20), 0));
        db.items.insert(112, item(item_type::HELMET, 100, None, 2));
        db.items.insert(114, item(item_type::HELMET, 200, None, 4));
        db
    }

    #[test]
    fn wears_the_best_owned_piece_and_bags_the_old_one() {
        let db = db();
        let mut world = FieldWorld::new();
        world.party = vec![0];
        world.equip(0, 0, Some(1));
        world.equip(0, 1, Some(114));
        world.items.insert(2, 1);
        world.items.insert(112, 1);
        let changes = equip_best(&mut world, &db, &[0]);
        assert_eq!(changes, vec![(0, 0, 2)]);
        assert_eq!(world.equipped(0)[0], Some(2));
        assert_eq!(
            world.equipped(0)[1],
            Some(114),
            "the weaker helmet stays in the bag"
        );
        assert_eq!(world.items.get(&1), Some(&1));
        assert_eq!(world.items.get(&2), None);
    }

    #[test]
    fn buys_what_beats_the_gear_while_the_gil_lasts() {
        let db = db();
        let mut world = FieldWorld::new();
        world.party = vec![0, 1];
        world.gil = 1000;
        world.equip(0, 0, Some(1));
        world.equip(1, 0, Some(1));
        let bought = buy_best(&mut world, &db, &[1, 2, 3, 114], &[0, 1]);
        // Member 0 takes the 900 gil weapon and a 200 helmet would overspend.
        assert_eq!(bought, vec![3]);
        assert_eq!(world.gil, 100);
        assert_eq!(world.equipped(0)[0], Some(3));
        assert_eq!(world.equipped(1)[0], Some(1));
        assert_eq!(world.items.get(&1), Some(&1));
    }

    #[test]
    fn stocks_medicine_in_turn_up_to_the_counts() {
        let mut db = db();
        db.items.insert(236, item(item_type::ITEM, 50, None, 0));
        db.items.insert(240, item(item_type::ITEM, 150, None, 0));
        let mut world = FieldWorld::new();
        world.gil = 500;
        world.items.insert(240, 19);
        let bought = buy_supplies(&mut world, &db, &[236, 240]);
        // One Phoenix Down tops it up to 20; the rest goes on Potions.
        assert_eq!(bought, vec![240, 236, 236, 236, 236, 236, 236, 236]);
        assert_eq!(world.gil, 0);
        assert_eq!(world.items.get(&240), Some(&20));
        assert_eq!(world.items.get(&236), Some(&7));
    }
}
