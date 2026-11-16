//! Party Setting rules (`Global/PartySettingUI.cs`) and how the screen is
//! opened: the `PARTYMENU` event opcode and the main menu entry
//! (`Global/Event/Engine/EventEngine.DoEventCode.cs:2282-2308`,
//! `Global/UI/UIKey/UIKeyTrigger.cs:980-1011`).
//!
//! The "locks" are not event variables read by the screen: the script that
//! opens the menu passes a minimum size and a bit mask of locked characters
//! (`PartyInfo::from_event`). The only event-variable hook in the pinned
//! source is the main menu hotfix in [`super::main_menu`].

use std::collections::BTreeSet;

use super::{BATTLE_END_IN_MENU, CHARACTER_NONE};
use crate::saves::state::PartyState;

/// Party member slots (`FF9StateGlobal.party.member`).
pub const PARTY_SLOTS: usize = 4;
/// Outside-party slots visible at once (`DisplayOutsideCharacters`
/// loops 0..8, `PartySettingUI.cs:364`).
pub const VISIBLE_SELECT_SLOTS: usize = 8;

/// One character of `FF9.PlayerList` as the menu setup reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RosterEntry {
    /// `info.slot_no` (a `CharacterId`).
    pub slot_no: u8,
    /// `info.party != 0`: the character has joined the roster
    /// (`FF9Play_Add`, `ff9play.cs:205-208`).
    pub available: bool,
}

/// `FF9PARTY_INFO` (`Global/ff9/FF9PARTY_INFO.cs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyInfo {
    /// Minimum party size (`party_ct`).
    pub party_ct: i32,
    /// Exact required size, or -1 for "at least `party_ct`".
    pub exact_party_ct: i32,
    /// The four party slots; [`CHARACTER_NONE`] is empty.
    pub menu: [u8; PARTY_SLOTS],
    /// Outside characters in the order the roster lists them, padded with
    /// [`CHARACTER_NONE`] to 8 plus 4 per extra floor.
    pub select: Vec<u8>,
    /// Characters that cannot be moved or swapped.
    pub fix: BTreeSet<u8>,
}

/// A place a character can stand on the screen. `Select` indices are
/// absolute (`4 * floor + column`): the source adds the current floor when
/// it reads or writes the slot (`PartySettingUI.cs:465-484`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartySlot {
    Menu(usize),
    Select(usize),
}

/// The first confirm: a picked slot and who stood there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub slot: PartySlot,
    pub id: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyError {
    /// The character on that slot is in `fix` (the beep, `PartySettingUI.cs:222-224`).
    Locked(u8),
    /// A slot index past the arrays.
    BadSlot,
    /// A character the caller cannot describe (the source would throw on a
    /// null `GetPlayer`).
    UnknownCharacter(u8),
}

/// Hit points and status of a character for the exit check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberHealth {
    pub hp: u32,
    /// `PLAYER.status` as `BattleStatus` bits.
    pub status: u64,
}

/// The result of a swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveReport {
    /// `ImpactfulActionCount++` happened: either side was a party slot
    /// (`PartySettingUI.cs:483-484`).
    pub impactful: bool,
}

/// Battle context for the menu opened from battle
/// (`UIKeyTrigger.cs:1005-1009`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleContext {
    /// `FF9.party.MemberCount`.
    pub member_count: i32,
    /// `UIManager.Battle.GetNonSwappableCharacters()`; computed by the
    /// battle HUD, not ported.
    pub non_swappable: BTreeSet<u8>,
}

impl PartyInfo {
    /// The `PARTYMENU` opcode (`EventEngine.DoEventCode.cs:2282-2308`).
    ///
    /// `party_ct` is arg 1, `fixed_mask` arg 2, `roster` is `PlayerList` in
    /// order, `members` the current party (use [`CHARACTER_NONE`] for an
    /// empty slot). `fix` gets every roster character whose bit
    /// `1 << slot_no` is set, whether or not that character has joined.
    /// `select` is the joined characters in roster order minus those in the
    /// party. `exact_party_ct` is -1.
    pub fn from_event(
        party_ct: i32,
        fixed_mask: i32,
        roster: &[RosterEntry],
        members: [u8; PARTY_SLOTS],
    ) -> Self {
        let mut select: Vec<u8> = roster
            .iter()
            .filter(|p| p.available)
            .map(|p| p.slot_no)
            .collect();
        let fix = roster
            .iter()
            .filter(|p| fixed_mask & 1i32.wrapping_shl(u32::from(p.slot_no)) != 0)
            .map(|p| p.slot_no)
            .collect();
        for &m in &members {
            if m != CHARACTER_NONE {
                remove_first(&mut select, m);
            }
        }
        Self {
            party_ct,
            exact_party_ct: -1,
            menu: members,
            select,
            fix,
        }
    }

    /// `UISceneHelper.GetCurrentPartyForMenu` (`UIKeyTrigger.cs:980-1011`):
    /// the Party entry of the main menu and the battle party swap.
    /// `party_ct` is `min(4, joined count)`, counted before the party is
    /// removed from the list. In battle the size must match exactly and the
    /// battle HUD's locked set applies.
    pub fn from_current_party(
        roster: &[RosterEntry],
        members: [u8; PARTY_SLOTS],
        battle: Option<&BattleContext>,
    ) -> Self {
        let joined = roster.iter().filter(|p| p.available).count();
        let mut info = Self::from_event(joined.min(4) as i32, 0, roster, members);
        if let Some(b) = battle {
            info.exact_party_ct = b.member_count;
            info.fix = b.non_swappable.clone();
        }
        info
    }

    /// `TotalCharCount` (`PartySettingUI.cs:103-114`).
    pub fn total_chars(&self) -> i32 {
        let menu = self.menu.iter().filter(|&&c| c != CHARACTER_NONE).count();
        let sel = self.select.iter().filter(|&&c| c != CHARACTER_NONE).count();
        (menu + sel) as i32
    }

    /// `FloorMax = max(0, (TotalCharCount - 6) / 4)` (`PartySettingUI.cs:115`).
    /// Integer division truncates toward zero, as in C#.
    pub fn floor_max(&self) -> i32 {
        ((self.total_chars() - 6) / 4).max(0)
    }

    /// `ExtendSelectSlots` (`PartySettingUI.cs:510-528`): the outside list
    /// is padded with [`CHARACTER_NONE`] (or cut) to `8 + 4 * floor_max`.
    /// A list that was never set starts as 8 empty slots; an empty `Vec`
    /// is the same case.
    pub fn extend_select_slots(&mut self) {
        let want = VISIBLE_SELECT_SLOTS + 4 * self.floor_max() as usize;
        self.select.resize(want, CHARACTER_NONE);
    }

    /// Next floor on a Down press, if there is one (`OnKeyNavigate`,
    /// `PartySettingUI.cs:186-212`). The source only allows changing floors
    /// while the cursor is in the outside panel.
    pub fn floor_down(&self, floor: i32) -> i32 {
        if floor < self.floor_max() {
            floor + 1
        } else {
            floor
        }
    }

    /// Previous floor on an Up press.
    pub fn floor_up(&self, floor: i32) -> i32 {
        if floor > 0 { floor - 1 } else { floor }
    }

    /// `info.fix.Contains(id)` for a real character.
    pub fn is_fixed(&self, id: u8) -> bool {
        id != CHARACTER_NONE && self.fix.contains(&id)
    }

    /// The character on a slot, [`CHARACTER_NONE`] when empty.
    pub fn char_at(&self, slot: PartySlot) -> Result<u8, PartyError> {
        match slot {
            PartySlot::Menu(i) => self.menu.get(i).copied(),
            PartySlot::Select(i) => self.select.get(i).copied(),
        }
        .ok_or(PartyError::BadSlot)
    }

    /// First confirm (`OnKeyConfirm`, `PartySettingUI.cs:214-238`): picking
    /// a locked character beeps; an empty slot can be picked (to move
    /// someone into it).
    pub fn select_slot(&self, slot: PartySlot) -> Result<Selection, PartyError> {
        let id = self.char_at(slot)?;
        if self.is_fixed(id) {
            return Err(PartyError::Locked(id));
        }
        Ok(Selection { slot, id })
    }

    /// Second confirm (`PartySettingUI.cs:239-253`, `SwapCharacter`,
    /// `:465-486`): the picked character goes to the target slot, whoever
    /// stood there goes to the picked slot. A locked occupant of the target
    /// beeps and nothing moves. Moving onto the picked slot itself changes
    /// nothing.
    pub fn move_to(&mut self, sel: Selection, target: PartySlot) -> Result<MoveReport, PartyError> {
        let char2 = self.char_at(target)?;
        self.char_at(sel.slot)?;
        if self.is_fixed(char2) {
            return Err(PartyError::Locked(char2));
        }
        self.set_slot(target, sel.id);
        self.set_slot(sel.slot, char2);
        Ok(MoveReport {
            impactful: matches!(sel.slot, PartySlot::Menu(_))
                || matches!(target, PartySlot::Menu(_)),
        })
    }

    fn set_slot(&mut self, slot: PartySlot, id: u8) {
        match slot {
            PartySlot::Menu(i) => self.menu[i] = id,
            PartySlot::Select(i) => self.select[i] = id,
        }
    }

    /// `FF9Party_Check` (`PartySettingUI.cs:420-436`): whether the screen
    /// may be left. Counts party members; one counts as healthy when its HP
    /// is above 0 and it has neither Petrify nor Venom
    /// ([`BATTLE_END_IN_MENU`]). The size must equal `exact_party_ct` when
    /// that is not negative, else reach `party_ct`; at least one member must
    /// be healthy either way.
    pub fn can_leave(
        &self,
        health: impl Fn(u8) -> Option<MemberHealth>,
    ) -> Result<bool, PartyError> {
        let mut healthy = 0;
        let mut count = 0;
        for &id in &self.menu {
            if id == CHARACTER_NONE {
                continue;
            }
            count += 1;
            let h = health(id).ok_or(PartyError::UnknownCharacter(id))?;
            if h.hp != 0 && h.status & BATTLE_END_IN_MENU == 0 {
                healthy += 1;
            }
        }
        Ok(if self.exact_party_ct >= 0 {
            count == self.exact_party_ct && healthy != 0
        } else {
            count >= self.party_ct && healthy != 0
        })
    }

    /// The four `FF9Play_SetParty(i, menu[i])` calls on exit
    /// (`PartySettingUI.cs:265-268`). A member left out of the party keeps
    /// its roster flag: `FF9Play_SetParty` never clears `info.party`.
    pub fn final_party(&self) -> [Option<u8>; PARTY_SLOTS] {
        self.menu.map(|c| (c != CHARACTER_NONE).then_some(c))
    }

    /// The help line key (`DisplayHelpInfo`, `PartySettingUI.cs:315-338`):
    /// `PartyHelp{n}Desp` for `party_ct` 1 to 4, and the 4 text otherwise.
    pub fn help_text_key(&self) -> &'static str {
        match self.party_ct {
            1 => "PartyHelp1Desp",
            2 => "PartyHelp2Desp",
            3 => "PartyHelp3Desp",
            _ => "PartyHelp4Desp",
        }
    }

    /// Writes the result into the save state: the four party slots, and
    /// `info.party = 1` on each chosen member (`FF9Play_Add` inside
    /// `FF9Play_SetParty`, `ff9play.cs:150-165`). Fails before changing
    /// anything if a chosen member is not in `characters`.
    pub fn apply_to_state(&self, state: &mut PartyState) -> Result<(), PartyError> {
        let mut idx = [None; PARTY_SLOTS];
        for (i, &c) in self.menu.iter().enumerate() {
            if c == CHARACTER_NONE {
                continue;
            }
            idx[i] = Some(
                state
                    .characters
                    .iter()
                    .position(|ch| ch.info.slot_no == c)
                    .ok_or(PartyError::UnknownCharacter(c))?,
            );
        }
        state.members = self.final_party();
        for k in idx.into_iter().flatten() {
            state.characters[k].info.party = 1;
        }
        Ok(())
    }
}

/// The roster as the setup code reads it from a saved party: one entry per
/// stored character, in stored order (`info.slot_no`, `info.party != 0`).
/// The stored order is the save's slot order; the source's `PlayerList`
/// order is not independently checked here.
pub fn roster_from_state(state: &PartyState) -> Vec<RosterEntry> {
    state
        .characters
        .iter()
        .map(|c| RosterEntry {
            slot_no: c.info.slot_no,
            available: c.info.party != 0,
        })
        .collect()
}

/// Current party slots as menu ids.
pub fn members_from_state(state: &PartyState) -> [u8; PARTY_SLOTS] {
    state.members.map(|m| m.unwrap_or(CHARACTER_NONE))
}

/// `List<T>.Remove` (first match only).
fn remove_first(v: &mut Vec<u8>, id: u8) {
    if let Some(i) = v.iter().position(|&x| x == id) {
        v.remove(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saves::ff9_tree::Resources;
    use crate::saves::state::{CharacterBasis, CharacterInfo, CharacterState};

    const ZIDANE: u8 = 0;
    const VIVI: u8 = 1;
    const GARNET: u8 = 2;
    const STEINER: u8 = 3;
    const FREYA: u8 = 4;
    const QUINA: u8 = 5;
    const EIKO: u8 = 6;
    const AMARANT: u8 = 7;
    const N: u8 = CHARACTER_NONE;

    fn roster(joined: &[u8]) -> Vec<RosterEntry> {
        (0..9)
            .map(|i| RosterEntry {
                slot_no: i,
                available: joined.contains(&i),
            })
            .collect()
    }

    fn ok(hp: u32) -> Option<MemberHealth> {
        Some(MemberHealth { hp, status: 0 })
    }

    /// `PARTYMENU` (`DoEventCode.cs:2282-2308`): min size 2, Zidane locked
    /// (mask bit 0) and Garnet locked (bit 2): mask 0b101 = 5. Roster has
    /// Zidane, Vivi, Garnet, Freya, Quina joined; party is Zidane, Vivi.
    /// Select = joined in roster order minus the party = Garnet, Freya, Quina.
    #[test]
    fn event_setup() {
        let info = PartyInfo::from_event(2, 0b101, &roster(&[0, 1, 2, 4, 5]), [ZIDANE, VIVI, N, N]);
        assert_eq!(info.party_ct, 2);
        assert_eq!(info.exact_party_ct, -1);
        assert_eq!(info.menu, [ZIDANE, VIVI, N, N]);
        assert_eq!(info.select, vec![GARNET, FREYA, QUINA]);
        assert_eq!(info.fix, BTreeSet::from([ZIDANE, GARNET]));
        // The mask applies to the whole roster, joined or not: bit 7 locks
        // Amarant before he joins.
        let r: Vec<RosterEntry> = (0..8)
            .map(|i| RosterEntry {
                slot_no: i,
                available: i < 2,
            })
            .collect();
        let info = PartyInfo::from_event(1, 1 << 7, &r, [ZIDANE, N, N, N]);
        assert_eq!(info.fix, BTreeSet::from([AMARANT]));
    }

    /// Main menu entry (`UIKeyTrigger.cs:980-1011`): party_ct is
    /// `min(4, joined)` counted before the party leaves the list; no locks;
    /// in battle the size is exact and the HUD's locked set is used.
    #[test]
    fn current_party_setup() {
        let r = roster(&[0, 1, 2, 3, 4, 5]);
        let info = PartyInfo::from_current_party(&r, [ZIDANE, VIVI, GARNET, STEINER], None);
        assert_eq!(info.party_ct, 4);
        assert_eq!(info.exact_party_ct, -1);
        assert!(info.fix.is_empty());
        assert_eq!(info.select, vec![FREYA, QUINA]);
        let r = roster(&[0, 1]);
        let info = PartyInfo::from_current_party(&r, [ZIDANE, N, VIVI, N], None);
        assert_eq!(info.party_ct, 2);
        let b = BattleContext {
            member_count: 3,
            non_swappable: BTreeSet::from([VIVI]),
        };
        let info = PartyInfo::from_current_party(
            &roster(&[0, 1, 2, 4]),
            [ZIDANE, VIVI, GARNET, N],
            Some(&b),
        );
        assert_eq!(info.exact_party_ct, 3);
        assert_eq!(info.fix, BTreeSet::from([VIVI]));
    }

    /// Floors: `FloorMax = max(0, (total - 6) / 4)` (`PartySettingUI.cs:115`).
    /// Totals 9 or fewer give floor 0 ((9-6)/4 = 0); 10 gives 1; 14 gives 2;
    /// 2 gives (2-6)/4 = -1 truncated toward zero = -1, clamped to 0.
    #[test]
    fn floors() {
        let mk = |n: usize| {
            let mut i = PartyInfo {
                party_ct: 1,
                exact_party_ct: -1,
                menu: [N; 4],
                select: vec![0; n],
                fix: BTreeSet::new(),
            };
            i.extend_select_slots();
            i
        };
        assert_eq!(mk(2).floor_max(), 0);
        assert_eq!(mk(9).floor_max(), 0);
        assert_eq!(mk(10).floor_max(), 1);
        assert_eq!(mk(14).floor_max(), 2);
        // ExtendSelectSlots: 8 + 4 * floor, padded with NONE.
        let i = mk(10);
        assert_eq!(i.select.len(), 12);
        assert_eq!(i.select[10], N);
        assert_eq!(mk(3).select.len(), 8);
        let i = mk(10);
        assert_eq!(i.floor_down(0), 1);
        assert_eq!(i.floor_down(1), 1);
        assert_eq!(i.floor_up(1), 0);
        assert_eq!(i.floor_up(0), 0);
    }

    /// A normal swap: Vivi (party slot 1) with Freya (outside slot 0).
    /// Both land swapped; party slot involved so impactful.
    #[test]
    fn swap_with_outside() {
        let mut info = PartyInfo::from_event(1, 0, &roster(&[0, 1, 4]), [ZIDANE, VIVI, N, N]);
        info.extend_select_slots();
        assert_eq!(info.select[0], FREYA);
        let sel = info.select_slot(PartySlot::Menu(1)).unwrap();
        assert_eq!(sel.id, VIVI);
        let r = info.move_to(sel, PartySlot::Select(0)).unwrap();
        assert!(r.impactful);
        assert_eq!(info.menu, [ZIDANE, FREYA, N, N]);
        assert_eq!(info.select[0], VIVI);
    }

    /// Moving into an empty party slot leaves NONE behind; moving between
    /// two outside slots is not impactful (`PartySettingUI.cs:483-484`).
    #[test]
    fn empty_slots_and_outside_moves() {
        let mut info = PartyInfo::from_event(1, 0, &roster(&[0, 1, 4]), [ZIDANE, N, N, N]);
        info.extend_select_slots(); // select = Vivi, Freya, NONE...
        let sel = info.select_slot(PartySlot::Select(0)).unwrap();
        let r = info.move_to(sel, PartySlot::Menu(2)).unwrap();
        assert!(r.impactful);
        assert_eq!(info.menu, [ZIDANE, N, VIVI, N]);
        assert_eq!(info.select[0], N);
        let sel = info.select_slot(PartySlot::Select(1)).unwrap();
        let r = info.move_to(sel, PartySlot::Select(5)).unwrap();
        assert!(!r.impactful);
        assert_eq!(info.select[1], N);
        assert_eq!(info.select[5], FREYA);
        // Picking an empty slot is allowed and moving onto it again is a no-op.
        let sel = info.select_slot(PartySlot::Menu(3)).unwrap();
        assert_eq!(sel.id, N);
        let before = info.clone();
        info.move_to(sel, PartySlot::Menu(3)).unwrap();
        assert_eq!(info, before);
    }

    /// Locked characters: picking one beeps, and so does targeting a slot
    /// they stand on (`PartySettingUI.cs:222,242`). Nothing moves.
    #[test]
    fn locks() {
        let mut info = PartyInfo::from_event(1, 0b1, &roster(&[0, 1, 4]), [ZIDANE, VIVI, N, N]);
        info.extend_select_slots();
        assert_eq!(
            info.select_slot(PartySlot::Menu(0)),
            Err(PartyError::Locked(ZIDANE))
        );
        let sel = info.select_slot(PartySlot::Menu(1)).unwrap();
        let before = info.clone();
        assert_eq!(
            info.move_to(sel, PartySlot::Menu(0)),
            Err(PartyError::Locked(ZIDANE))
        );
        assert_eq!(info, before);
        assert_eq!(
            info.move_to(sel, PartySlot::Select(99)),
            Err(PartyError::BadSlot)
        );
        assert_eq!(
            info.select_slot(PartySlot::Menu(9)),
            Err(PartyError::BadSlot)
        );
        // NONE is never "fixed" even when the set is odd.
        assert!(!info.is_fixed(N));
    }

    /// `FF9Party_Check` (`PartySettingUI.cs:420-436`). Min size 2, at least
    /// one healthy member; exact size overrides.
    #[test]
    fn leave_rules() {
        let mut info = PartyInfo::from_event(2, 0, &roster(&[0, 1, 2]), [ZIDANE, VIVI, N, N]);
        assert_eq!(info.can_leave(|_| ok(100)), Ok(true));
        // One member only: below the minimum.
        info.menu = [ZIDANE, N, N, N];
        assert_eq!(info.can_leave(|_| ok(100)), Ok(false));
        // Both members KO'd: nobody healthy.
        info.menu = [ZIDANE, VIVI, N, N];
        assert_eq!(info.can_leave(|_| ok(0)), Ok(false));
        // Petrified Zidane does not count as healthy, Vivi does.
        let h = |id: u8| {
            Some(MemberHealth {
                hp: 50,
                status: if id == ZIDANE { 1 } else { 0 },
            })
        };
        assert_eq!(info.can_leave(h), Ok(true));
        // Venom (bit 1) also disqualifies; Stop (bit 12) does not.
        let h = |id: u8| {
            Some(MemberHealth {
                hp: 50,
                status: if id == ZIDANE { 2 } else { 1 << 12 },
            })
        };
        assert_eq!(info.can_leave(h), Ok(true));
        let all_venom = |_: u8| Some(MemberHealth { hp: 50, status: 2 });
        assert_eq!(info.can_leave(all_venom), Ok(false));
        // Exact size 3: two members is not enough, three is.
        info.exact_party_ct = 3;
        assert_eq!(info.can_leave(|_| ok(10)), Ok(false));
        info.menu = [ZIDANE, VIVI, GARNET, N];
        assert_eq!(info.can_leave(|_| ok(10)), Ok(true));
        // Unknown character.
        assert_eq!(
            info.can_leave(|id| if id == GARNET { None } else { ok(10) }),
            Err(PartyError::UnknownCharacter(GARNET))
        );
    }

    #[test]
    fn help_keys() {
        let mut i = PartyInfo::from_event(2, 0, &roster(&[0]), [ZIDANE, N, N, N]);
        assert_eq!(i.help_text_key(), "PartyHelp2Desp");
        i.party_ct = 0;
        assert_eq!(i.help_text_key(), "PartyHelp4Desp");
        i.party_ct = 1;
        assert_eq!(i.help_text_key(), "PartyHelp1Desp");
    }

    fn character(slot: u8, party: u8) -> CharacterState {
        CharacterState {
            name: String::new(),
            category: 0,
            level: 1,
            exp: 0,
            cur: Resources::default(),
            max: Resources::default(),
            equip: [255; 5],
            status: 0,
            trance: 0,
            web_bone: 0,
            ability_ap: vec![],
            support_abilities: [0, 0],
            info: CharacterInfo {
                slot_no: slot,
                party,
                ..CharacterInfo::default()
            },
            basis: CharacterBasis::default(),
            bonus_cap: 0,
        }
    }

    fn state() -> PartyState {
        PartyState {
            members: [Some(0), None, None, None],
            characters: (0..5).map(|i| character(i, u8::from(i < 3))).collect(),
            gil: 0,
            items: vec![],
            rare_item_flags: vec![],
            escape_no: 0,
            summon_flag: 0,
            frog_no: 0,
            steal_no: 0,
            dragon_no: 0,
        }
    }

    /// Full flow on a saved party: roster and members read from the state,
    /// Garnet (joined, outside) swaps into slot 1, the result is written
    /// back; Freya (never joined, never offered) stays unjoined.
    #[test]
    fn state_round_trip() {
        let mut st = state();
        let r = roster_from_state(&st);
        assert!(r[2].available && !r[3].available);
        let mut info = PartyInfo::from_current_party(&r, members_from_state(&st), None);
        info.extend_select_slots();
        assert_eq!(info.select[..3], [VIVI, GARNET, N]);
        let sel = info.select_slot(PartySlot::Select(1)).unwrap();
        info.move_to(sel, PartySlot::Menu(1)).unwrap();
        info.apply_to_state(&mut st).unwrap();
        assert_eq!(st.members, [Some(ZIDANE), Some(GARNET), None, None]);
        assert_eq!(st.characters[3].info.party, 0);
        // A removed member keeps its roster flag (`FF9Play_SetParty` never clears).
        let mut info2 =
            PartyInfo::from_current_party(&roster_from_state(&st), members_from_state(&st), None);
        info2.extend_select_slots();
        let sel = info2.select_slot(PartySlot::Menu(1)).unwrap();
        info2.move_to(sel, PartySlot::Select(5)).unwrap();
        info2.apply_to_state(&mut st).unwrap();
        assert_eq!(st.members, [Some(ZIDANE), None, None, None]);
        assert_eq!(st.characters[2].info.party, 1);
        // Unknown member: nothing is written.
        let before = st.clone();
        info2.menu[2] = EIKO;
        assert_eq!(
            info2.apply_to_state(&mut st),
            Err(PartyError::UnknownCharacter(EIKO))
        );
        assert_eq!(st, before);
    }
}
