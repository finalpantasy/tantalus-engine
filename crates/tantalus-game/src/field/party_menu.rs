//! The party selection screen a field script opens with `PARTYMENU` (0xB2,
//! `EventEngine.DoEventCode.cs` `case PARTYMENU`): `EventService.OpenPartyMenu` shows
//! `PartySettingUI`, and the script waits until the player leaves it with a valid party
//! (`PartySettingUI.OnKeyCancel` -> `FF9Party_Check` -> `FF9Play_SetParty` for the four slots).
//!
//! The rules are `tantalus_legacy::party_menu::party::PartyInfo`; this keeps the open screen in
//! the field world and writes the result back to the party.

use super::world::FieldWorld;
use tantalus_legacy::party_menu::CHARACTER_NONE;
use tantalus_legacy::party_menu::party::{
    MemberHealth, PARTY_SLOTS, PartyError, PartyInfo, PartySlot, RosterEntry,
};

/// `FF9.PlayerList`: the twelve playable characters, by `slot_no`.
const ROSTER_SIZE: u8 = 12;

impl FieldWorld {
    /// `PARTYMENU minimum fixed`: builds `FF9PARTY_INFO` from the roster and the current party.
    /// A character is on the roster (`info.party != 0`) when the last `FULLMEMBER` mask has its
    /// bit, or, before any `FULLMEMBER`, when it is in the party.
    pub fn open_party_menu_for(&mut self, minimum: i32, fixed_mask: i32) {
        let mut members = [CHARACTER_NONE; PARTY_SLOTS];
        for (slot, character) in members.iter_mut().zip(&self.party) {
            *slot = u8::try_from(*character).unwrap_or(CHARACTER_NONE);
        }
        let roster: Vec<RosterEntry> = (0..ROSTER_SIZE)
            .map(|slot_no| RosterEntry {
                slot_no,
                available: members.contains(&slot_no)
                    || self
                        .party_reserve
                        .is_some_and(|mask| mask & (1 << slot_no) != 0),
            })
            .collect();
        self.party_menu = Some(PartyInfo::from_event(minimum, fixed_mask, &roster, members));
        self.open_party_menu();
    }

    /// The open party screen, if any.
    pub fn party_menu_info(&self) -> Option<&PartyInfo> {
        self.party_menu.as_ref()
    }

    /// `FF9Party_Check`: whether the screen's party may be confirmed.
    pub fn party_menu_can_leave(&self) -> bool {
        self.party_menu
            .as_ref()
            .is_some_and(|info| info.can_leave(|id| self.member_health(id)).unwrap_or(false))
    }

    /// One swap on the open screen (`PartySettingUI.OnItemSelect` twice): the character on
    /// `from` trades places with whoever is on `to`. Locked characters refuse.
    pub fn party_menu_move(&mut self, from: PartySlot, to: PartySlot) -> Result<(), PartyError> {
        let info = self.party_menu.as_mut().ok_or(PartyError::BadSlot)?;
        let selection = info.select_slot(from)?;
        info.move_to(selection, to)?;
        Ok(())
    }

    /// Leaves the screen (`OnKeyCancel`): when `FF9Party_Check` passes, the four slots become
    /// the party (`FF9Play_SetParty`; an empty slot is dropped) and the waiting script goes
    /// on. Returns false, leaving the screen open, when the party is not allowed.
    pub fn confirm_party_menu(&mut self) -> bool {
        if !self.party_menu_can_leave() {
            return false;
        }
        let Some(info) = self.party_menu.take() else {
            return false;
        };
        self.party = info
            .final_party()
            .iter()
            .flatten()
            .map(|id| i32::from(*id))
            .collect();
        self.close_party_menu();
        true
    }

    /// What a player who only wants to go on picks: the party as it stands when that is allowed,
    /// else the first roster characters moved into the empty slots until it is.
    pub fn confirm_party_menu_default(&mut self) -> bool {
        if self.confirm_party_menu() {
            return true;
        }
        let Some(info) = self.party_menu.as_ref() else {
            return false;
        };
        let empty: Vec<usize> = (0..PARTY_SLOTS)
            .filter(|slot| info.menu[*slot] == CHARACTER_NONE)
            .collect();
        let waiting: Vec<usize> = (0..info.select.len())
            .filter(|slot| info.select[*slot] != CHARACTER_NONE)
            .collect();
        for (slot, from) in empty.into_iter().zip(waiting) {
            if self
                .party_menu_move(PartySlot::Select(from), PartySlot::Menu(slot))
                .is_err()
            {
                continue;
            }
            if self.confirm_party_menu() {
                return true;
            }
        }
        false
    }

    fn member_health(&self, id: u8) -> Option<MemberHealth> {
        let stats = self.character_stats.get(&i32::from(id));
        Some(MemberHealth {
            // A character the field has no record for counts as healthy (new-game defaults).
            hp: stats.map_or(1, |stats| stats.current_hp.max(0) as u32),
            // The field keeps no lasting battle statuses yet, so nobody is petrified or
            // poisoned (`BATTLE_END_IN_MENU`) here.
            status: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_party_is_kept_when_it_is_large_enough() {
        let mut world = FieldWorld::new();
        world.party = vec![0, 1, 2];
        world.party_reserve = Some(0b1_1111);
        world.open_party_menu_for(3, 1);
        let info = world.party_menu_info().unwrap();
        assert_eq!(
            info.select
                .iter()
                .filter(|id| **id != CHARACTER_NONE)
                .count(),
            2
        );
        assert!(info.fix.contains(&0));
        assert!(world.party_menu_open());
        assert!(world.confirm_party_menu_default());
        assert!(!world.party_menu_open());
        assert_eq!(world.party, vec![0, 1, 2]);
    }

    #[test]
    fn a_small_party_is_filled_from_the_roster() {
        let mut world = FieldWorld::new();
        world.party = vec![0];
        world.party_reserve = Some(0b1011);
        world.open_party_menu_for(3, 0);
        assert!(!world.confirm_party_menu());
        assert!(world.party_menu_open());
        assert!(world.confirm_party_menu_default());
        assert_eq!(world.party, vec![0, 1, 3]);
    }

    #[test]
    fn a_locked_member_cannot_be_swapped_out() {
        let mut world = FieldWorld::new();
        world.party = vec![0, 1];
        world.party_reserve = Some(0b111);
        world.open_party_menu_for(2, 1);
        assert_eq!(
            world.party_menu_move(PartySlot::Menu(0), PartySlot::Select(0)),
            Err(PartyError::Locked(0))
        );
        world
            .party_menu_move(PartySlot::Menu(1), PartySlot::Select(0))
            .unwrap();
        assert!(world.confirm_party_menu());
        assert_eq!(world.party, vec![0, 2]);
    }
}
