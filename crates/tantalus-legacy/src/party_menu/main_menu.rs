//! Main menu rules that belong to the screens in this module
//! (`Global/MainMenuUI.cs`): which entries are enabled, which character
//! slots a screen accepts, and the Order swap.

use std::collections::BTreeSet;

/// `MainMenuUI.SubMenu` (`MainMenuUI.cs:815-826`), in enum order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubMenu {
    Item,
    Ability,
    Equip,
    Status,
    Order,
    Card,
    Config,
    Party,
    None,
}

impl SubMenu {
    /// `Enum.GetValues(typeof(SubMenu))` order (`MainMenuUI.cs:369-376`).
    pub const ALL: [SubMenu; 9] = [
        Self::Item,
        Self::Ability,
        Self::Equip,
        Self::Status,
        Self::Order,
        Self::Card,
        Self::Config,
        Self::Party,
        Self::None,
    ];

    /// `subMenu.ToString()`, the key `EnabledSubMenus` is matched with.
    pub fn name(self) -> &'static str {
        match self {
            Self::Item => "Item",
            Self::Ability => "Ability",
            Self::Equip => "Equip",
            Self::Status => "Status",
            Self::Order => "Order",
            Self::Card => "Card",
            Self::Config => "Config",
            Self::Party => "Party",
            Self::None => "None",
        }
    }
}

/// `fldMapNo` of the Palace hall the hotfix names (`MainMenuUI.cs:360`).
pub const HOTFIX_FIELD: i32 = 2207;
/// `ScenarioCounter` the hotfix names (`MainMenuUI.cs:360`).
pub const HOTFIX_SCENARIO: i32 = 9840;

/// `MainMenuUI.IsSubMenuEnabled` (`MainMenuUI.cs:358-367`).
///
/// `enabled` is `EnabledSubMenus`: names a script restricted the menu to;
/// empty means everything. Party is always off in field 2207 at scenario
/// 9840 (the source's hotfix for Zidane returning with the Gulug Stone).
/// "Ability" also opens for the names "ActiveAbility" or
/// "SupportingAbility". Whether the Party entry exists at all in the menu
/// depends on `Configuration.Hacks.AllCharactersAvailable` or
/// `Configuration.Battle.IsMenuEnabledInBattle` (`MainMenuUI.cs:682`), not
/// modeled.
pub fn is_enabled(
    menu: SubMenu,
    enabled: &BTreeSet<String>,
    fld_map_no: i32,
    scenario_counter: i32,
) -> bool {
    if menu == SubMenu::Party && fld_map_no == HOTFIX_FIELD && scenario_counter == HOTFIX_SCENARIO {
        return false;
    }
    if enabled.is_empty() {
        return true;
    }
    if menu == SubMenu::Ability
        && (enabled.contains("ActiveAbility") || enabled.contains("SupportingAbility"))
    {
        return true;
    }
    enabled.contains(menu.name())
}

/// `GetFirstAvailableSubMenu` (`MainMenuUI.cs:369-376`): the first enabled
/// entry in enum order; `None` when nothing matches (the loop reaches the
/// `None` entry too, so an empty restriction set answers `Item`).
pub fn first_available(
    enabled: &BTreeSet<String>,
    fld_map_no: i32,
    scenario_counter: i32,
) -> SubMenu {
    SubMenu::ALL
        .into_iter()
        .find(|&m| is_enabled(m, enabled, fld_map_no, scenario_counter))
        .unwrap_or(SubMenu::None)
}

/// Whether confirming a character slot opens the screen
/// (`MainMenuUI.cs:222-262`): Ability, Equip and Status need a member in
/// the slot; Order works on empty slots as well.
pub fn can_pick_character(menu: SubMenu, member_present: bool) -> bool {
    match menu {
        SubMenu::Order => true,
        SubMenu::Ability | SubMenu::Equip | SubMenu::Status => member_present,
        _ => false,
    }
}

/// A party member as the Order swap sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderMember {
    pub id: u8,
    /// `info.row`: 0 front, 1 back (`ToggleOrder` flips bit 0,
    /// `MainMenuUI.cs:603-621`).
    pub row: u8,
}

/// A slot index past the four party slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadSlot;

/// `ToggleOrder` (`MainMenuUI.cs:603-621`): picking the same slot twice
/// flips that member's row (an empty slot does nothing); picking two
/// slots swaps them, empty slots included. The action always counts as
/// impactful (`ImpactfulActionCount++`), so this returns `true`.
pub fn toggle_order(
    party: &mut [Option<OrderMember>; 4],
    current: usize,
    target: usize,
) -> Result<bool, BadSlot> {
    if current >= 4 || target >= 4 {
        return Err(BadSlot);
    }
    if current == target {
        if let Some(m) = party[current].as_mut() {
            m.row ^= 1;
        }
    } else {
        party.swap(current, target);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// `IsSubMenuEnabled` (`MainMenuUI.cs:358-367`): an empty set enables
    /// everything except the hotfixed Party; a named set enables only its
    /// names; the ability names open Ability.
    #[test]
    fn enabling() {
        let none = BTreeSet::new();
        assert!(is_enabled(SubMenu::Item, &none, 100, 0));
        assert!(is_enabled(SubMenu::Party, &none, 100, 0));
        // Hotfix: Palace hall at scenario 9840.
        assert!(!is_enabled(SubMenu::Party, &none, 2207, 9840));
        assert!(is_enabled(SubMenu::Party, &none, 2207, 9839));
        assert!(is_enabled(SubMenu::Item, &none, 2207, 9840));
        let only = set(&["Item", "Status"]);
        assert!(is_enabled(SubMenu::Item, &only, 100, 0));
        assert!(!is_enabled(SubMenu::Equip, &only, 100, 0));
        assert!(!is_enabled(SubMenu::Party, &set(&["Party"]), 2207, 9840));
        let ab = set(&["SupportingAbility"]);
        assert!(is_enabled(SubMenu::Ability, &ab, 100, 0));
        assert!(!is_enabled(SubMenu::Item, &ab, 100, 0));
    }

    /// `GetFirstAvailableSubMenu` walks enum order.
    #[test]
    fn first_entry() {
        assert_eq!(first_available(&BTreeSet::new(), 1, 0), SubMenu::Item);
        assert_eq!(
            first_available(&set(&["Status", "Config"]), 1, 0),
            SubMenu::Status
        );
        assert_eq!(first_available(&set(&["Party"]), 2207, 9840), SubMenu::None);
        assert_eq!(first_available(&set(&["Bogus"]), 1, 0), SubMenu::None);
        assert_eq!(SubMenu::Order.name(), "Order");
    }

    /// Order menu: swap, flip row, empty slots.
    #[test]
    fn order_rules() {
        let a = OrderMember { id: 0, row: 0 };
        let b = OrderMember { id: 1, row: 1 };
        let mut p = [Some(a), Some(b), None, None];
        assert_eq!(toggle_order(&mut p, 0, 1), Ok(true));
        assert_eq!(p[..2], [Some(b), Some(a)]);
        // Same slot: row flips (0 -> 1, then 1 -> 0).
        assert_eq!(toggle_order(&mut p, 1, 1), Ok(true));
        assert_eq!(p[1], Some(OrderMember { id: 0, row: 1 }));
        assert_eq!(toggle_order(&mut p, 1, 1), Ok(true));
        assert_eq!(p[1], Some(a));
        // Into an empty slot.
        assert_eq!(toggle_order(&mut p, 0, 3), Ok(true));
        assert_eq!(p, [None, Some(a), None, Some(b)]);
        // Same empty slot: nothing changes but it still counts.
        assert_eq!(toggle_order(&mut p, 2, 2), Ok(true));
        assert_eq!(p, [None, Some(a), None, Some(b)]);
        assert_eq!(toggle_order(&mut p, 4, 0), Err(BadSlot));
    }

    #[test]
    fn character_picking() {
        assert!(can_pick_character(SubMenu::Order, false));
        assert!(!can_pick_character(SubMenu::Status, false));
        assert!(can_pick_character(SubMenu::Equip, true));
        assert!(!can_pick_character(SubMenu::Item, true));
    }
}
