//! Item list, key items and the 99 stack cap.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use super::data::ItemDb;
use super::{ITEM_COUNT_MAX, NO_ITEM};

/// `FF9ITEM` (`Global/ff9/FF9ITEM.cs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub id: u16,
    pub count: u8,
}

/// `FF9StateGlobal.item` plus `rare_item_obtained` and `rare_item_used`.
///
/// The slot list is a growable list in the pinned source
/// (`Memoria/Data/ff9item.2.cs:258-266`); the old fixed 256-slot array is
/// commented out (`:29`). Regular ids are 0..=255, so unique stacks never
/// exceed 256 unless a mod defines more items. No other cap exists in source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    pub slots: Vec<Slot>,
    pub key_obtained: BTreeSet<i32>,
    pub key_used: BTreeSet<i32>,
}

/// Failure of the generic (regular, key, card) entry points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenericError {
    /// Card ids (item id % 1000 in 512..612) go to the Tetra Master card
    /// database (`QuadMistDatabase`), not ported here.
    CardNotPorted,
    /// `id % 1000` is 612 or more: the source returns 0 and does nothing.
    Unknown,
}

impl Inventory {
    /// `FF9Item_GetPtr` (`ff9item.2.cs:231-234`): first slot with count > 0.
    fn find(&self, id: u16) -> Option<usize> {
        self.slots.iter().position(|s| s.count > 0 && s.id == id)
    }

    /// `FF9Item_GetCount` (`ff9item.2.cs:236-242`).
    pub fn count(&self, id: u16) -> i32 {
        self.find(id).map_or(0, |i| i32::from(self.slots[i].count))
    }

    /// `FF9Item_Add` (`ff9item.2.cs:244-269`). Returns the count added.
    /// Negative counts are not defined by the source (it casts to `Byte`),
    /// so they add nothing here.
    pub fn add(&mut self, id: u16, count: i32) -> i32 {
        if id == NO_ITEM || count < 0 {
            return 0;
        }
        let mut count = count.min(ITEM_COUNT_MAX);
        if let Some(i) = self.find(id) {
            let have = i32::from(self.slots[i].count);
            if have + count > ITEM_COUNT_MAX {
                count = ITEM_COUNT_MAX - have;
            }
            self.slots[i].count = (have + count) as u8;
            return count;
        }
        if let Some(s) = self.slots.iter_mut().find(|s| s.count == 0) {
            s.id = id;
            s.count = count as u8;
            return count;
        }
        self.slots.push(Slot {
            id,
            count: count as u8,
        });
        count
    }

    /// `FF9Item_Remove` (`ff9item.2.cs:271-282`). Returns the count removed
    /// (clamped to what is held). Negative counts remove nothing.
    pub fn remove(&mut self, id: u16, count: i32) -> i32 {
        if count < 0 {
            return 0;
        }
        match self.find(id) {
            Some(i) => {
                let n = count.min(i32::from(self.slots[i].count));
                self.slots[i].count -= n as u8;
                n
            }
            None => 0,
        }
    }

    /// Key item add (`FF9Item_AddImportant`, `ff9item.2.cs:317-321`).
    pub fn add_important(&mut self, id: i32) {
        self.key_obtained.insert(id);
        self.key_used.remove(&id);
    }

    /// `FF9Item_RemoveImportant` (`ff9item.2.cs:323-327`).
    pub fn remove_important(&mut self, id: i32) {
        self.key_obtained.remove(&id);
        self.key_used.remove(&id);
    }

    /// `FF9Item_UseImportant` (`ff9item.2.cs:329-332`).
    pub fn use_important(&mut self, id: i32) {
        self.key_used.insert(id);
    }

    /// `FF9Item_UnuseImportant` (`ff9item.2.cs:334-337`).
    pub fn unuse_important(&mut self, id: i32) {
        self.key_used.remove(&id);
    }

    /// `FF9Item_IsExistImportant` (`ff9item.2.cs:339-342`).
    pub fn has_important(&self, id: i32) -> bool {
        self.key_obtained.contains(&id)
    }

    /// `FF9Item_IsUsedImportant` (`ff9item.2.cs:344-347`).
    pub fn is_used_important(&self, id: i32) -> bool {
        self.key_used.contains(&id)
    }

    /// `FF9Item_Add_Generic` (`ff9item.2.cs:181-200`). Item ids use
    /// `id % 1000`: 0..256 regular, 256..512 key item, 512..612 card
    /// (`:413-428`). Returns the count added (key items: 1 or 0).
    pub fn add_generic(&mut self, id: i32, count: i32) -> Result<i32, GenericError> {
        match id.rem_euclid(1000) {
            m if m < 256 => Ok(self.add(regular_id(id), count)),
            256..=511 => {
                if count <= 0 {
                    return Ok(0);
                }
                self.add_important(important_id(id));
                Ok(1)
            }
            512..=611 => Err(GenericError::CardNotPorted),
            _ => Err(GenericError::Unknown),
        }
    }

    /// `FF9Item_Remove_Generic` (`ff9item.2.cs:202-218`).
    pub fn remove_generic(&mut self, id: i32, count: i32) -> Result<i32, GenericError> {
        match id.rem_euclid(1000) {
            m if m < 256 => Ok(self.remove(regular_id(id), count)),
            256..=511 => {
                let k = important_id(id);
                if count > 0 && self.has_important(k) {
                    self.remove_important(k);
                    Ok(1)
                } else {
                    Ok(0)
                }
            }
            512..=611 => Err(GenericError::CardNotPorted),
            _ => Err(GenericError::Unknown),
        }
    }

    /// `FF9Item_GetCount_Generic` (`ff9item.2.cs:220-229`).
    pub fn count_generic(&self, id: i32) -> Result<i32, GenericError> {
        match id.rem_euclid(1000) {
            m if m < 256 => Ok(self.count(regular_id(id))),
            256..=511 => Ok(i32::from(self.has_important(important_id(id)))),
            512..=611 => Err(GenericError::CardNotPorted),
            _ => Err(GenericError::Unknown),
        }
    }

    /// `ItemUI.ArrangeAuto` (`Global/ItemUI.cs:934-947`): drop empty slots,
    /// then order by `sort`, ties by item id. The source comparator is
    /// total for unique ids, so the result is deterministic.
    pub fn arrange_auto(&mut self, db: &ItemDb) {
        self.slots.retain(|s| s.id != NO_ITEM && s.count > 0);
        self.slots.sort_by(|a, b| {
            if a.id == b.id {
                return Ordering::Equal;
            }
            let sa = db.get(a.id).map_or(0.0, |d| d.sort);
            let sb = db.get(b.id).map_or(0.0, |d| d.sort);
            if sa > sb || (sa == sb && a.id > b.id) {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        });
    }

    /// Slots with count > 0 in list order, as the sell list shows them
    /// (`Global/ShopUI.cs:897-910`).
    pub fn held(&self) -> impl Iterator<Item = &Slot> {
        self.slots.iter().filter(|s| s.count > 0)
    }
}

/// `GetRegularIdFromItemId` for pool 0 .. n (`ff9item.2.cs:430-437`).
fn regular_id(item_id: i32) -> u16 {
    let v = (item_id / 1000) * 256 + item_id % 1000;
    u16::try_from(v).unwrap_or(NO_ITEM)
}

/// `GetImportantIdFromItemId` (`ff9item.2.cs:446-453`).
fn important_id(item_id: i32) -> i32 {
    (item_id / 1000) * 256 + item_id % 1000 - 256
}
