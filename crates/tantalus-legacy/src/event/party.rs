//! Party slots of an event script: `ObjTable.player_link` and
//! `EventEngine.SetupPartyUID`.
//!
//! Scripts name the party by the local uids 251-254. `InitObject` (NEW3)
//! and `GetUIDFromLocalUID` turn slot `uid - 251` into the entry stored in
//! `partyUID`, which `SetupPartyUID` builds at field start, after party
//! changes, and on `SYNCPARTY` (0xE9). Character ids are `CharacterId`
//! values (0 Zidane to 7 Amarant, 8-10 Cinna, Marcus and Blank, 11 Beatrix).
//!
//! While Eiko is abducted (`EventState.IsEikoAbducted`, scenario 9860 to
//! 9989) the lookup skips the entries linked to her.

/// First party-slot local uid (`partyUID[0]`).
pub const PARTY_SLOT_FIRST: u8 = 251;
/// Last party-slot local uid (`partyUID[3]`).
pub const PARTY_SLOT_LAST: u8 = 254;

/// `SetupPartyUID`'s priority order: Zidane, Eiko, Steiner, Vivi, Freya,
/// the others (8-10), Quina, Amarant, Garnet, Beatrix. `None` is the
/// "others" group.
const ORDER: [Option<u8>; 10] = [
    Some(0),
    Some(6),
    Some(3),
    Some(1),
    Some(4),
    None,
    Some(5),
    Some(7),
    Some(2),
    Some(11),
];

/// Eiko's `CharacterId`.
const EIKO: u8 = 6;

/// `EventState.IsEikoAbducted` (EventState.cs:36): from Kuja taking Eiko in
/// the Desert Palace (9860) until her rescue (9990).
pub fn is_eiko_abducted(scenario: i32) -> bool {
    (9860..9990).contains(&scenario)
}

/// Characters after Amarant other than Beatrix (Cinna, Marcus, Blank).
fn is_other(character: u8) -> bool {
    character > 7 && character != 11
}

/// Vanilla `ObjTable.ReadData` `player_link` of entry `index` in a script of
/// `entry_count` entries: the last entry links to Beatrix (11), the eight
/// before it to characters 0-7, the rest to no one.
pub fn player_link(entry_count: usize, index: usize) -> Option<u8> {
    if index >= entry_count {
        None
    } else if index + 1 == entry_count {
        Some(11)
    } else if index + 9 >= entry_count {
        Some((index + 9 - entry_count) as u8)
    } else {
        None
    }
}

/// One script entry as `SetupPartyUID` sees it: its `player_link` and
/// whether its code is non-empty (`ObjTable.size > 0`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EntryLink {
    pub character: Option<u8>,
    pub has_code: bool,
}

/// Vanilla links for a script whose entries have the given code flags.
pub fn vanilla_links(has_code: impl ExactSizeIterator<Item = bool>) -> Vec<EntryLink> {
    let count = has_code.len();
    has_code
        .enumerate()
        .map(|(index, has_code)| EntryLink {
            character: player_link(count, index),
            has_code,
        })
        .collect()
}

/// `EventEngineUtils.GetEventCharacterSIdHacked`: the entry linked to
/// `character`, else the `member`-th entry (in priority order) linked to a
/// character outside `party`, so a slot still gets an object when the
/// field has no entry for that member. With `avoid_eiko` (`IsEikoAbducted`)
/// no entry linked to Eiko is picked (EventEngineUtils.cs:1870-1878).
fn character_entry(
    entries: &[EntryLink],
    party: &[u8],
    character: u8,
    member: usize,
    avoid_eiko: bool,
) -> Option<usize> {
    let allowed = |link: u8| link != EIKO || !avoid_eiko;
    if let Some(index) = entries.iter().position(|entry| {
        entry.has_code && entry.character == Some(character) && allowed(character)
    }) {
        return Some(index);
    }
    let mut left = member;
    for order in ORDER {
        for (index, entry) in entries.iter().enumerate() {
            let Some(link) = entry.character else {
                continue;
            };
            let fits = match order {
                Some(wanted) => link == wanted,
                None => is_other(link),
            };
            if entry.has_code && fits && allowed(link) && !party.contains(&link) {
                if left == 0 {
                    return Some(index);
                }
                left -= 1;
            }
        }
    }
    None
}

/// `SetupPartyUID`: the entry behind each party slot for the party
/// `party` (character ids in slot order); `avoid_eiko` is `IsEikoAbducted`.
pub fn party_uid(entries: &[EntryLink], party: &[u8], avoid_eiko: bool) -> [Option<usize>; 4] {
    let party: Vec<u8> = party.iter().take(4).copied().collect();
    let mut slots = [None; 4];
    let mut member = 0;
    let mut place = |character: u8, member: &mut usize| {
        if *member < 4
            && let Some(entry) = character_entry(entries, &party, character, *member, avoid_eiko)
        {
            slots[*member] = Some(entry);
            *member += 1;
        }
    };
    for order in ORDER {
        match order {
            None => {
                for &character in &party {
                    if is_other(character) {
                        place(character, &mut member);
                    }
                }
            }
            Some(character) if party.contains(&character) => place(character, &mut member),
            Some(_) => {}
        }
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(count: usize) -> Vec<EntryLink> {
        vanilla_links(std::iter::repeat_n(true, count))
    }

    #[test]
    fn vanilla_links_end_with_characters_then_beatrix() {
        assert_eq!(player_link(20, 10), None);
        assert_eq!(player_link(20, 11), Some(0));
        assert_eq!(player_link(20, 18), Some(7));
        assert_eq!(player_link(20, 19), Some(11));
        assert_eq!(player_link(20, 20), None);
        assert_eq!(player_link(2, 0), Some(7));
    }

    #[test]
    fn slots_follow_priority_order_not_party_order() {
        // 20 entries: Zidane is entry 11, Vivi 12, Garnet 13, Steiner 14.
        let table = links(20);
        // Party order Vivi, Garnet, Zidane, Steiner -> Zidane, Steiner,
        // Vivi, Garnet.
        assert_eq!(
            party_uid(&table, &[1, 2, 0, 3], false),
            [Some(11), Some(14), Some(12), Some(13)]
        );
        assert_eq!(party_uid(&table, &[], false), [None; 4]);
    }

    #[test]
    fn a_member_without_code_borrows_an_unused_character_entry() {
        let mut table = links(20);
        // Freya's entry (15) is empty; she is in the party with Zidane.
        table[15].has_code = false;
        // Zidane -> 11. Freya has no entry, so slot 1 takes the member-1th
        // entry linked to a non-member in priority order: Eiko (17) is the
        // 0th, Steiner (14) the 1st.
        assert_eq!(
            party_uid(&table, &[0, 4], false),
            [Some(11), Some(14), None, None]
        );
    }

    #[test]
    fn eiko_abducted_spans_9860_to_9989() {
        assert!(!is_eiko_abducted(9859));
        assert!(is_eiko_abducted(9860));
        assert!(is_eiko_abducted(9989));
        assert!(!is_eiko_abducted(9990));
    }

    #[test]
    fn an_abducted_eiko_s_entries_are_skipped() {
        let mut table = links(20);
        // Freya's entry (15) is empty; party Zidane and Freya. Unabducted, the
        // borrowed entry is the 1st non-member in priority order, Steiner (14),
        // after Eiko (17); abducted, Eiko is skipped and the 1st is Vivi (12).
        table[15].has_code = false;
        assert_eq!(
            party_uid(&table, &[0, 4], true),
            [Some(11), Some(12), None, None]
        );
        // Eiko in the party gets no entry of her own while abducted: slot 1
        // borrows the 1st unused non-member entry in priority order (Vivi 12
        // is the 0th, Freya 15 the 1st; Steiner is a member).
        let table = links(20);
        assert_eq!(
            party_uid(&table, &[0, 6, 3], false),
            [Some(11), Some(17), Some(14), None]
        );
        assert_eq!(
            party_uid(&table, &[0, 6, 3], true),
            [Some(11), Some(15), Some(14), None]
        );
    }
}
