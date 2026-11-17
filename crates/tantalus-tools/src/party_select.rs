//! Pad control of the field party screen (`PartySettingUI`) that `PARTYMENU` opens: a cursor
//! over the four party slots then the waiting characters, Confirm to pick a character and
//! Confirm again on another place to swap them (`OnItemSelect`, `SwapCharacter`), Cancel to
//! leave once the party is allowed (`OnKeyCancel` -> `FF9Party_Check`).
//!
//! The rules and the result live in `tantalus_game::field::party_menu`; this only turns pad
//! presses into moves.

use tantalus_game::FieldWorld;
use tantalus_legacy::party_menu::party::{PARTY_SLOTS, PartySlot};

use crate::field_session::CONFIRM;

/// `EventInput.Cancel`.
pub const CANCEL: i32 = 0x10000;
const LEFT: i32 = 0x80;
const RIGHT: i32 = 0x20;
const UP: i32 = 0x10;
const DOWN: i32 = 0x40;

/// The cursor and the picked place, if any.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PartySelect {
    pub cursor: usize,
    pub picked: Option<usize>,
}

/// What a press did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartySelectEvent {
    Moved,
    Picked,
    Swapped,
    /// The press was refused (a locked character, or leaving with a party that is not allowed).
    Refused,
    /// The screen closed and the party was written back.
    Closed,
}

/// Place `index` on the screen: 0 to 3 the party, then the waiting characters.
pub fn slot_at(index: usize) -> PartySlot {
    if index < PARTY_SLOTS {
        PartySlot::Menu(index)
    } else {
        PartySlot::Select(index - PARTY_SLOTS)
    }
}

impl PartySelect {
    /// One tick of presses while the screen is open; `None` when nothing happened.
    pub fn press(&mut self, world: &mut FieldWorld, pressed: i32) -> Option<PartySelectEvent> {
        let places = PARTY_SLOTS + world.party_menu_info()?.select.len();
        if pressed & CANCEL != 0 {
            if self.picked.take().is_some() {
                return Some(PartySelectEvent::Moved);
            }
            if world.confirm_party_menu() {
                *self = Self::default();
                return Some(PartySelectEvent::Closed);
            }
            return Some(PartySelectEvent::Refused);
        }
        if pressed & CONFIRM != 0 {
            let Some(from) = self.picked.take() else {
                // `PartySettingUI.OnKeyConfirm`: a locked character beeps.
                if world
                    .party_menu_info()?
                    .select_slot(slot_at(self.cursor))
                    .is_err()
                {
                    return Some(PartySelectEvent::Refused);
                }
                self.picked = Some(self.cursor);
                return Some(PartySelectEvent::Picked);
            };
            return Some(
                match world.party_menu_move(slot_at(from), slot_at(self.cursor)) {
                    Ok(()) => PartySelectEvent::Swapped,
                    Err(_) => PartySelectEvent::Refused,
                },
            );
        }
        let step: isize = match () {
            _ if pressed & (LEFT | UP) != 0 => -1,
            _ if pressed & (RIGHT | DOWN) != 0 => 1,
            _ => return None,
        };
        self.cursor = (self.cursor as isize + step).rem_euclid(places as isize) as usize;
        Some(PartySelectEvent::Moved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_a_waiting_character_in_and_leave() {
        let mut world = FieldWorld::new();
        world.party = vec![0, 1];
        world.party_reserve = Some(0b111);
        world.open_party_menu_for(2, 1);
        let mut select = PartySelect::default();
        // Zidane (slot 0) is locked.
        assert_eq!(
            select.press(&mut world, CONFIRM),
            Some(PartySelectEvent::Refused)
        );
        select.press(&mut world, RIGHT);
        assert_eq!(
            select.press(&mut world, CONFIRM),
            Some(PartySelectEvent::Picked)
        );
        for _ in 0..3 {
            select.press(&mut world, RIGHT);
        }
        assert_eq!(select.cursor, 4);
        assert_eq!(
            select.press(&mut world, CONFIRM),
            Some(PartySelectEvent::Swapped)
        );
        assert_eq!(
            select.press(&mut world, CANCEL),
            Some(PartySelectEvent::Closed)
        );
        assert_eq!(world.party, vec![0, 2]);
        assert!(!world.party_menu_open());
    }

    #[test]
    fn leaving_with_too_few_members_is_refused() {
        let mut world = FieldWorld::new();
        world.party = vec![0];
        world.party_reserve = Some(0b11);
        world.open_party_menu_for(2, 0);
        let mut select = PartySelect::default();
        assert_eq!(
            select.press(&mut world, CANCEL),
            Some(PartySelectEvent::Refused)
        );
        assert!(world.party_menu_open());
    }
}
