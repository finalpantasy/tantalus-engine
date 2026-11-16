//! Item menu rules (`Global/ItemUI.cs`): list rows, use on party members,
//! multi-target toggling, arranging, key items and Gysahl Greens.
//!
//! The field-use math is [`crate::shop::use_item`]; this module adds the
//! menu flow around it. Menu flow facts that differ from a plain loop over
//! `use_item` are called out where they occur.

use std::collections::BTreeMap;

use crate::battle::targeting::target_type;
use crate::shop::data::item_type;
use crate::shop::{
    FieldEffect, FieldItemError, FieldTarget, Inventory, ItemDb, ItemUseOutcome, NO_ITEM, use_item,
};

/// `FF9FITEM_RARE_NONE` (`ItemUI.cs:45`): the placeholder row of an empty
/// key item list.
pub const KEY_ITEM_NONE: i32 = 255;
/// `FF9FITEM_CHOCOBO_NONE` (`ItemUI.cs:46`).
pub const CHOCOBO_NONE: u8 = 0;
/// `FF9FITEM_EVENT_VEGETABLE` (`ItemUI.cs:47`): `gEventGlobal` index.
pub const EVENT_VEGETABLE: usize = 181;
/// `FF9FITEM_EVENT_KIND_CHOCOBO` (`ItemUI.cs:48`): `gEventGlobal` index.
pub const EVENT_KIND_CHOCOBO: usize = 191;
/// `FF9StateSystem.mode` value Gysahl Greens needs (`ItemUI.cs:960`: the
/// check is `mode != 3`). The mode numbers are raw in
/// [`crate::saves::ModeBlock`]; 3 is the value the source tests.
pub const MODE_WORLD: u8 = 3;
/// `RegularItem.GysahlGreens` (`ItemUI.cs:323`), also `field_item.rs`.
pub const GYSAHL_GREENS: u16 = 251;

/// `TargetDisplay` (`Memoria/Data/Battle/TargetDisplay.cs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetDisplay {
    None,
    Hp,
    Mp,
    Debuffs,
    Buffs,
}

impl TargetDisplay {
    /// Raw byte (`TargetDisplay.cs:5-9`).
    pub fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::None,
            1 => Self::Hp,
            2 => Self::Mp,
            3 => Self::Debuffs,
            4 => Self::Buffs,
            _ => return None,
        })
    }

    /// Which panels a target card shows (`ItemUI.DisplayTarget`,
    /// `ItemUI.cs:782-815`): `(hp_mp_panels, status_panel)`.
    pub fn panels(self) -> (bool, bool) {
        match self {
            Self::None | Self::Hp | Self::Mp => (true, false),
            Self::Debuffs | Self::Buffs => (false, true),
        }
    }
}

/// `ITEM_DATA` as the menu reads it: `info.Target`, `info.DisplayStats`
/// and the field effect row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemEffectInfo {
    /// `TargetType` byte, see [`target_type`].
    pub target: u8,
    pub display: TargetDisplay,
    pub effect: FieldEffect,
}

/// Item id to its effect row. An item with no entry has `effect_id < 0`, so
/// `ff9item.HasItemEffect` is false (`Memoria/Data/ff9item.2.cs:488-491`).
#[derive(Debug, Clone, Default)]
pub struct ItemEffects {
    pub effects: BTreeMap<u16, ItemEffectInfo>,
}

impl ItemEffects {
    pub fn get(&self, id: u16) -> Option<&ItemEffectInfo> {
        self.effects.get(&id)
    }

    /// `ff9item.HasItemEffect`.
    pub fn has_effect(&self, id: u16) -> bool {
        self.effects.contains_key(&id)
    }
}

/// One row of the item list (`FieldItemListData`, `ItemUI.cs:1089-1093`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemRow {
    pub id: u16,
    pub count: u8,
    /// `Enable`: the item has the `Usable` type bit. Spent rows are false.
    pub enabled: bool,
}

/// One row of the key item list (`DisplayKeyItemDetail`, `ItemUI.cs:750-780`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyItemRow {
    /// [`KEY_ITEM_NONE`] for the placeholder row.
    pub id: i32,
    /// The "new" exclamation icon: shown until the item has been opened.
    pub is_new: bool,
}

/// How the "Arrange" dialog choices map (`ItemUI.cs:289`:
/// `_currentArrangeMode = go.transform.GetSiblingIndex() + 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrangeMode {
    /// Choice 0 (mode 1): `ArrangeAuto`.
    Auto,
    /// Choice 1 (mode 2): pick two stacks and swap them.
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemMenuError {
    /// The item has no row in the item table.
    UnknownItem,
    /// The single target slot is empty or past slot 3.
    BadTarget,
    /// Manual arrange index out of range.
    BadIndex,
    /// The field math refused (see [`FieldItemError`]).
    Field(FieldItemError),
}

impl From<FieldItemError> for ItemMenuError {
    fn from(e: FieldItemError) -> Self {
        Self::Field(e)
    }
}

/// What the confirm key on a usable item row starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseStart {
    /// Beep (`FF9SFX_Play(102)`): no effect row, not `Usable`, or already
    /// spent this visit (`ItemUI.cs:319`).
    Rejected,
    /// Gysahl Greens skips the target list (`ItemUI.cs:321-333`).
    GysahlGreens,
    /// Open the target list.
    Target(TargetPlan),
}

/// The target list setup (`ItemUI.cs:334-364`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetPlan {
    pub target: u8,
    pub display: TargetDisplay,
    /// `CanToggleMulti`: the bumpers switch single and all targets.
    pub can_multi: bool,
    /// The list opens with every member selected (`SetMultipleTarget(...)`
    /// for All, AllAlly, AllEnemy, Everyone; `ItemUI.cs:363`).
    pub start_multi: bool,
}

impl TargetPlan {
    /// `ToggleMultipleTarget` (`ItemUI.cs:926-931`): flips only when the
    /// item allows it.
    pub fn toggle(&self, multi: bool) -> bool {
        if self.can_multi { !multi } else { multi }
    }
}

/// `CanToggleMulti` (`ItemUI.cs:896-910`): many-target types, and more than
/// one party member present.
pub fn can_toggle_multi(target: u8, member_count: usize) -> bool {
    matches!(
        target,
        target_type::MANY_ANY | target_type::MANY_ALLY | target_type::MANY_ENEMY
    ) && member_count > 1
}

/// Which targets a use covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetChoice {
    /// Party slot 0 to 3.
    One(usize),
    /// Every non-empty slot (`_multiTarget`).
    All,
}

/// Result of using an item on the party.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseReport {
    /// At least one target was affected: one item was consumed.
    pub consumed: bool,
    /// `(party slot, outcome)` per target tried, in slot order.
    pub outcomes: Vec<(usize, ItemUseOutcome)>,
    /// Count left after consuming.
    pub remaining: i32,
    /// The stack ran out: the row stays in the list, greyed, for this
    /// visit (`ItemUI.cs:427-440`).
    pub exhausted: bool,
    /// `MainMenuUI.ImpactfulActionCount++` (`ItemUI.cs:429`).
    pub impactful: bool,
}

/// The item tables a use needs.
#[derive(Debug, Clone, Copy)]
pub struct ItemTables<'a> {
    pub db: &'a ItemDb,
    pub effects: &'a ItemEffects,
}

/// Item menu state: the ids spent this visit (`_usedItemIdList`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemMenu {
    pub used: Vec<u16>,
}

impl ItemMenu {
    /// `_usedItemIdList.Clear()` (`ItemUI.cs:216`, on Hide, and
    /// `:590` when the list panel is shown again).
    pub fn reset(&mut self) {
        self.used.clear();
    }

    /// `DisplayItem` (`ItemUI.cs:649-680`): stacks with a count in slot
    /// order, plus spent stacks of this visit shown with count 0. `Enable`
    /// is the `Usable` type bit alone.
    pub fn rows(&self, inv: &Inventory, db: &ItemDb) -> Vec<ItemRow> {
        let mut out = Vec::new();
        for s in &inv.slots {
            if s.count > 0 {
                let usable = db
                    .get(s.id)
                    .is_some_and(|d| d.kind & item_type::USABLE != 0);
                out.push(ItemRow {
                    id: s.id,
                    count: s.count,
                    enabled: usable,
                });
            } else if self.used.contains(&s.id) {
                out.push(ItemRow {
                    id: s.id,
                    count: 0,
                    enabled: false,
                });
            }
        }
        out
    }

    /// The confirm test (`ItemUI.cs:319`): effect row, `Usable`, not spent.
    pub fn can_use(&self, db: &ItemDb, fx: &ItemEffects, id: u16) -> bool {
        fx.has_effect(id)
            && db.get(id).is_some_and(|d| d.kind & item_type::USABLE != 0)
            && !self.used.contains(&id)
    }

    /// Confirm on a row in the Use list (`ItemUI.cs:319-364`).
    pub fn begin_use(
        &self,
        db: &ItemDb,
        fx: &ItemEffects,
        id: u16,
        member_count: usize,
    ) -> UseStart {
        if !self.can_use(db, fx, id) {
            return UseStart::Rejected;
        }
        if id == GYSAHL_GREENS {
            return UseStart::GysahlGreens;
        }
        let Some(info) = fx.get(id) else {
            return UseStart::Rejected;
        };
        UseStart::Target(TargetPlan {
            target: info.target,
            display: info.display,
            can_multi: can_toggle_multi(info.target, member_count),
            start_multi: matches!(
                info.target,
                target_type::ALL
                    | target_type::ALL_ALLY
                    | target_type::ALL_ENEMY
                    | target_type::EVERYONE
            ),
        })
    }

    /// Gysahl Greens (`ItemUI.cs:321-333`, `FF9FItem_Vegetable`,
    /// `:958-970`). Works only on the world map (`mode == 3`) when a chocobo
    /// kind is set (`gEventGlobal[191] != 0`), the vegetable flag is clear
    /// (`gEventGlobal[181] == 0`) and `ff9.w_frameChocoboCheck()` passes
    /// (`frame_check`, a world query not ported). On success the flag is set,
    /// one item is consumed and a spent stack stays greyed. The scene change
    /// and dialog after it are UI.
    pub fn use_gysahl_greens(
        &mut self,
        inv: &mut Inventory,
        mode: u8,
        event_global: &mut [u8],
        frame_check: bool,
    ) -> bool {
        let kind = event_global.get(EVENT_KIND_CHOCOBO).copied();
        let veg = event_global.get(EVENT_VEGETABLE).copied();
        let (Some(kind), Some(veg)) = (kind, veg) else {
            return false;
        };
        if mode != MODE_WORLD || kind == CHOCOBO_NONE || veg != 0 || !frame_check {
            return false;
        }
        event_global[EVENT_VEGETABLE] = 1;
        inv.remove(GYSAHL_GREENS, 1);
        if inv.count(GYSAHL_GREENS) == 0 {
            self.used.push(GYSAHL_GREENS);
        }
        true
    }

    /// Confirm on a target (`ItemUI.cs:410-445`). A single target runs the
    /// calculator once; the multi-target path runs it for every non-empty
    /// slot and succeeds if any did (`canUseAbility |= ...`, no
    /// short-circuit, `:424`). One item is consumed in either case, after all
    /// targets ran, so each run sees the stack as it was before the use
    /// (this matters for Gems, whose power is the held count). On failure
    /// nothing changes.
    ///
    /// `random8` is `Comn.random8`, drawn only by the revive script, once
    /// per target that reaches it. `use_item` consumes from the inventory it
    /// is given, so each run gets a scratch copy; the real stack loses one
    /// item here.
    pub fn use_on(
        &mut self,
        tables: ItemTables<'_>,
        inv: &mut Inventory,
        id: u16,
        party: &mut [Option<FieldTarget>; 4],
        choice: TargetChoice,
        random8: &mut dyn FnMut() -> u32,
    ) -> Result<UseReport, ItemMenuError> {
        let (db, fx) = (tables.db, tables.effects);
        let info = *fx.get(id).ok_or(ItemMenuError::UnknownItem)?;
        let slots: Vec<usize> = match choice {
            TargetChoice::One(i) => {
                if i >= 4 || party[i].is_none() {
                    return Err(ItemMenuError::BadTarget);
                }
                vec![i]
            }
            TargetChoice::All => (0..4).filter(|&i| party[i].is_some()).collect(),
        };
        let mut outcomes = Vec::new();
        let mut any = false;
        for i in slots {
            let Some(target) = party[i].as_mut() else {
                continue;
            };
            let mut scratch = inv.clone();
            let out = use_item(db, &mut scratch, id, &info.effect, target, &mut *random8)?;
            if matches!(out, ItemUseOutcome::Used { .. }) {
                any = true;
            }
            outcomes.push((i, out));
        }
        if !any {
            return Ok(UseReport {
                consumed: false,
                outcomes,
                remaining: inv.count(id),
                exhausted: false,
                impactful: false,
            });
        }
        inv.remove(id, 1);
        let remaining = inv.count(id);
        let exhausted = remaining == 0;
        if exhausted && !self.used.contains(&id) {
            self.used.push(id);
        }
        Ok(UseReport {
            consumed: true,
            outcomes,
            remaining,
            exhausted,
            impactful: true,
        })
    }
}

/// `ItemUI.ArrangeAuto` (`ItemUI.cs:934-947`); see
/// [`Inventory::arrange_auto`].
pub fn arrange_auto(inv: &mut Inventory, db: &ItemDb) {
    inv.arrange_auto(db);
}

/// Manual arrange swap (`ItemUI.cs:459-461`): the two raw stack slots swap
/// places. The indices are positions in the inventory list, not in the
/// displayed rows: the source passes the list indices of the displayed rows
/// straight to `FF9.item[...]`, so they differ whenever the list holds a
/// spent (count 0) slot that is not displayed. This function takes raw
/// slot indices and does not re-map them.
pub fn arrange_swap(inv: &mut Inventory, a: usize, b: usize) -> Result<(), ItemMenuError> {
    if a >= inv.slots.len() || b >= inv.slots.len() {
        return Err(ItemMenuError::BadIndex);
    }
    inv.slots.swap(a, b);
    Ok(())
}

/// `DisplayKeyItem` (`ItemUI.cs:725-740`): the obtained key items, or the
/// single placeholder row when there are none.
///
/// Order: the source enumerates a `HashSet<Int32>`
/// (`FF9StateGlobal.cs:975`), which .NET enumerates in insertion order only
/// while nothing was removed. [`Inventory`] keeps a sorted set, so this list
/// is ascending by id. The two differ whenever key items were obtained out
/// of id order.
pub fn key_item_rows(inv: &Inventory) -> Vec<KeyItemRow> {
    if inv.key_obtained.is_empty() {
        return vec![KeyItemRow {
            id: KEY_ITEM_NONE,
            is_new: false,
        }];
    }
    inv.key_obtained
        .iter()
        .map(|&id| KeyItemRow {
            id,
            is_new: !inv.is_used_important(id),
        })
        .collect()
}

/// Closing the key item description (`DisplayKeyItemSkin(false)`,
/// `ItemUI.cs:848`) marks the item used, which clears its "new" icon.
/// The placeholder row cannot be opened (`ItemUI.cs:391`).
pub fn open_key_item(inv: &mut Inventory, id: i32) -> bool {
    if id == KEY_ITEM_NONE || !inv.has_important(id) {
        return false;
    }
    inv.use_important(id);
    true
}

/// The empty-slot id the list skips (`ItemUI.cs:934-947`).
pub const EMPTY_SLOT_ID: u16 = NO_ITEM;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shop::{ItemData, Slot};

    const POTION: u16 = 236;
    const ETHER: u16 = 237;
    const PHOENIX: u16 = 249;
    const TENT: u16 = 245;
    const DAGGER: u16 = 1;
    const HI_POTION: u16 = 244;

    fn item(kind: u8, sort: f32) -> ItemData {
        ItemData {
            price: 0,
            selling_price: 0,
            equip: 0,
            kind,
            sort,
            eq_lv: 0.0,
            bonus: 0,
            ability: vec![],
            weapon_power: None,
            armor: None,
        }
    }

    fn db() -> ItemDb {
        let mut items = BTreeMap::new();
        let usable = item_type::ITEM | item_type::USABLE;
        items.insert(POTION, item(usable, 10.0));
        items.insert(ETHER, item(usable, 11.0));
        items.insert(PHOENIX, item(usable, 12.0));
        items.insert(TENT, item(usable, 13.0));
        items.insert(HI_POTION, item(usable, 10.5));
        items.insert(DAGGER, item(item_type::WEAPON, 1.0));
        items.insert(GYSAHL_GREENS, item(usable, 14.0));
        ItemDb {
            items,
            bonuses: vec![],
        }
    }

    fn fx() -> ItemEffects {
        let mut e = BTreeMap::new();
        let mk = |target, script_id, power| ItemEffectInfo {
            target,
            display: TargetDisplay::Hp,
            effect: FieldEffect {
                script_id,
                power,
                status: 0,
            },
        };
        e.insert(POTION, mk(target_type::SINGLE_ALLY, 69, 10));
        e.insert(ETHER, mk(target_type::SINGLE_ALLY, 70, 15));
        e.insert(PHOENIX, mk(target_type::SINGLE_ALLY, 72, 0));
        e.insert(TENT, mk(target_type::MANY_ALLY, 76, 0));
        e.insert(GYSAHL_GREENS, mk(target_type::SINGLE_ALLY, 0, 0));
        ItemEffects { effects: e }
    }

    fn member(hp: u32, max_hp: u32) -> FieldTarget {
        FieldTarget {
            hp,
            max_hp,
            mp: 0,
            max_mp: 50,
            status: 0,
            permanent_status: 0,
        }
    }

    fn inv(stacks: &[(u16, u8)]) -> Inventory {
        Inventory {
            slots: stacks
                .iter()
                .map(|&(id, count)| Slot { id, count })
                .collect(),
            ..Inventory::default()
        }
    }

    fn no_rng() -> impl FnMut() -> u32 {
        || panic!("random8 must not be drawn")
    }

    /// `DisplayItem` (`ItemUI.cs:649-680`): count > 0 rows show, with Enable
    /// from the Usable bit (the Dagger row is greyed); a zero-count slot
    /// shows only when it was spent this visit.
    #[test]
    fn list_rows() {
        let mut m = ItemMenu::default();
        let i = inv(&[(POTION, 3), (DAGGER, 1), (ETHER, 0), (PHOENIX, 0)]);
        m.used.push(PHOENIX);
        let rows = m.rows(&i, &db());
        assert_eq!(
            rows,
            vec![
                ItemRow {
                    id: POTION,
                    count: 3,
                    enabled: true
                },
                ItemRow {
                    id: DAGGER,
                    count: 1,
                    enabled: false
                },
                ItemRow {
                    id: PHOENIX,
                    count: 0,
                    enabled: false
                },
            ]
        );
        m.reset();
        assert_eq!(m.rows(&i, &db()).len(), 2);
    }

    /// Confirm test, `ItemUI.cs:319`: needs an effect row, the Usable bit and
    /// an unspent stack. Dagger has no effect row; a spent Potion is refused.
    #[test]
    fn confirm_rules() {
        let mut m = ItemMenu::default();
        let (d, f) = (db(), fx());
        assert!(m.can_use(&d, &f, POTION));
        assert!(!m.can_use(&d, &f, DAGGER));
        assert_eq!(m.begin_use(&d, &f, DAGGER, 4), UseStart::Rejected);
        m.used.push(POTION);
        assert_eq!(m.begin_use(&d, &f, POTION, 4), UseStart::Rejected);
        assert_eq!(
            m.begin_use(&d, &f, GYSAHL_GREENS, 4),
            UseStart::GysahlGreens
        );
    }

    /// `CanToggleMulti` (`ItemUI.cs:896-910`): Tent is ManyAlly, so with two
    /// members the bumpers toggle; with one member they do not. A SingleAlly
    /// Potion never toggles.
    #[test]
    fn multi_target_toggle() {
        let m = ItemMenu::default();
        let (d, f) = (db(), fx());
        let UseStart::Target(p) = m.begin_use(&d, &f, TENT, 2) else {
            panic!("tent targets")
        };
        assert!(p.can_multi && !p.start_multi);
        assert!(p.toggle(false));
        assert!(!p.toggle(true));
        let UseStart::Target(p) = m.begin_use(&d, &f, TENT, 1) else {
            panic!("tent targets")
        };
        assert!(!p.can_multi);
        assert!(!p.toggle(false));
        let UseStart::Target(p) = m.begin_use(&d, &f, POTION, 4) else {
            panic!("potion targets")
        };
        assert!(!p.can_multi && !p.start_multi);
        assert!(can_toggle_multi(target_type::MANY_ANY, 2));
        assert!(!can_toggle_multi(target_type::ALL, 4));
        // All, AllAlly, AllEnemy and Everyone start with everyone selected.
        let mut f2 = f.clone();
        f2.effects.get_mut(&POTION).unwrap().target = target_type::ALL_ALLY;
        let UseStart::Target(p) = m.begin_use(&d, &f2, POTION, 4) else {
            panic!("potion targets")
        };
        assert!(p.start_multi && !p.can_multi);
    }

    /// Potion script 69: HP += power * 10 = 100, clamped to max
    /// (`field_item.rs`). Hand check: 40 + 100 = 140 > 100 so the gain is 60;
    /// the stack goes 2 to 1; not exhausted.
    #[test]
    fn single_target_use() {
        let mut m = ItemMenu::default();
        let (d, f) = (db(), fx());
        let mut i = inv(&[(POTION, 2)]);
        let mut party = [Some(member(40, 100)), None, None, None];
        let r = m
            .use_on(
                ItemTables {
                    db: &d,
                    effects: &f,
                },
                &mut i,
                POTION,
                &mut party,
                TargetChoice::One(0),
                &mut no_rng(),
            )
            .unwrap();
        assert!(r.consumed && r.impactful && !r.exhausted);
        assert_eq!(r.remaining, 1);
        assert_eq!(party[0].unwrap().hp, 100);
        assert_eq!(
            r.outcomes,
            vec![(
                0,
                ItemUseOutcome::Used {
                    hp_gained: 60,
                    mp_gained: 0
                }
            )]
        );
        // Full HP: calculator misses, nothing is consumed (`CanBeHealed`).
        let r = m
            .use_on(
                ItemTables {
                    db: &d,
                    effects: &f,
                },
                &mut i,
                POTION,
                &mut party,
                TargetChoice::One(0),
                &mut no_rng(),
            )
            .unwrap();
        assert!(!r.consumed && !r.impactful);
        assert_eq!(i.count(POTION), 1);
        // Empty slot.
        assert_eq!(
            m.use_on(
                ItemTables {
                    db: &d,
                    effects: &f
                },
                &mut i,
                POTION,
                &mut party,
                TargetChoice::One(2),
                &mut no_rng()
            ),
            Err(ItemMenuError::BadTarget)
        );
    }

    /// Using the last item: the stack hits 0, the row stays greyed for the
    /// visit (`ItemUI.cs:427-440`), and a second confirm is refused.
    #[test]
    fn last_item_stays_listed() {
        let mut m = ItemMenu::default();
        let (d, f) = (db(), fx());
        let mut i = inv(&[(POTION, 1)]);
        let mut party = [Some(member(1, 100)), None, None, None];
        let r = m
            .use_on(
                ItemTables {
                    db: &d,
                    effects: &f,
                },
                &mut i,
                POTION,
                &mut party,
                TargetChoice::One(0),
                &mut no_rng(),
            )
            .unwrap();
        assert!(r.exhausted);
        assert_eq!(m.used, vec![POTION]);
        assert_eq!(
            m.rows(&i, &d),
            vec![ItemRow {
                id: POTION,
                count: 0,
                enabled: false
            }]
        );
        assert_eq!(m.begin_use(&d, &f, POTION, 4), UseStart::Rejected);
    }

    /// Multi-target: Tent (script 76) heals half of max HP and MP on every
    /// member who can be healed; one item is consumed for the whole use, not
    /// one per member. Hand check: max 100/50 gives +50 HP, +25 MP each.
    /// A member at full HP and MP misses, the use still counts because the
    /// others succeeded (`canUseAbility |= ...`, `ItemUI.cs:412-429`).
    #[test]
    fn multi_target_consumes_once() {
        let mut m = ItemMenu::default();
        let (d, f) = (db(), fx());
        let mut i = inv(&[(TENT, 5)]);
        let full = FieldTarget {
            hp: 100,
            max_hp: 100,
            mp: 50,
            max_mp: 50,
            status: 0,
            permanent_status: 0,
        };
        let mut party = [
            Some(member(10, 100)),
            Some(full),
            Some(member(20, 100)),
            None,
        ];
        let r = m
            .use_on(
                ItemTables {
                    db: &d,
                    effects: &f,
                },
                &mut i,
                TENT,
                &mut party,
                TargetChoice::All,
                &mut no_rng(),
            )
            .unwrap();
        assert!(r.consumed);
        assert_eq!(r.remaining, 4);
        assert_eq!(r.outcomes.len(), 3);
        assert_eq!(party[0].unwrap().hp, 60);
        assert_eq!(party[0].unwrap().mp, 25);
        assert_eq!(party[1].unwrap().hp, 100);
        assert_eq!(party[2].unwrap().hp, 70);
        assert_eq!(r.outcomes[1], (1, ItemUseOutcome::NoEffect));
        // Everyone full: no success, nothing consumed.
        let mut party = [Some(full), Some(full), None, None];
        let r = m
            .use_on(
                ItemTables {
                    db: &d,
                    effects: &f,
                },
                &mut i,
                TENT,
                &mut party,
                TargetChoice::All,
                &mut no_rng(),
            )
            .unwrap();
        assert!(!r.consumed);
        assert_eq!(i.count(TENT), 4);
    }

    /// Phoenix Down draws `random8` once per revived target: 1 + 7 % 10 = 8.
    #[test]
    fn revive_draws_once_per_target() {
        let mut m = ItemMenu::default();
        let (d, f) = (db(), fx());
        let mut i = inv(&[(PHOENIX, 2)]);
        let mut party = [Some(member(0, 100)), Some(member(0, 100)), None, None];
        let mut draws = 0;
        let mut rng = || {
            draws += 1;
            7
        };
        let r = m
            .use_on(
                ItemTables {
                    db: &d,
                    effects: &f,
                },
                &mut i,
                PHOENIX,
                &mut party,
                TargetChoice::One(1),
                &mut rng,
            )
            .unwrap();
        assert_eq!(draws, 1);
        assert_eq!(party[1].unwrap().hp, 8);
        assert_eq!(party[0].unwrap().hp, 0);
        assert_eq!(r.remaining, 1);
    }

    /// Gysahl Greens (`ItemUI.cs:958-970`): world mode 3, chocobo kind set,
    /// vegetable flag clear and the frame check; it sets `gEventGlobal[181]`.
    #[test]
    fn gysahl_greens() {
        let mut m = ItemMenu::default();
        let mut i = inv(&[(GYSAHL_GREENS, 1)]);
        let mut ev = vec![0u8; 2048];
        // No chocobo kind: refused, nothing consumed.
        assert!(!m.use_gysahl_greens(&mut i, MODE_WORLD, &mut ev, true));
        ev[EVENT_KIND_CHOCOBO] = 2;
        // Wrong mode and failed frame check also refuse.
        assert!(!m.use_gysahl_greens(&mut i, 2, &mut ev, true));
        assert!(!m.use_gysahl_greens(&mut i, MODE_WORLD, &mut ev, false));
        assert_eq!(i.count(GYSAHL_GREENS), 1);
        assert!(m.use_gysahl_greens(&mut i, MODE_WORLD, &mut ev, true));
        assert_eq!(ev[EVENT_VEGETABLE], 1);
        assert_eq!(i.count(GYSAHL_GREENS), 0);
        assert_eq!(m.used, vec![GYSAHL_GREENS]);
        // Flag already set: refused.
        i.add(GYSAHL_GREENS, 1);
        assert!(!m.use_gysahl_greens(&mut i, MODE_WORLD, &mut ev, true));
        // Short event buffer cannot hold the indices.
        assert!(!m.use_gysahl_greens(&mut i, MODE_WORLD, &mut [0u8; 8], true));
    }

    /// `ArrangeAuto` (`ItemUI.cs:934-947`): drop spent slots, sort by `sort`
    /// then id. Hi-Potion (10.5) lands between Potion (10) and Ether (11).
    #[test]
    fn arrange_auto_orders_by_sort() {
        let d = db();
        let mut i = inv(&[(ETHER, 2), (POTION, 0), (HI_POTION, 4), (POTION, 9)]);
        arrange_auto(&mut i, &d);
        let ids: Vec<u16> = i.slots.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![POTION, HI_POTION, ETHER]);
        assert_eq!(i.slots[0].count, 9);
    }

    /// Manual arrange swaps two raw slots (`ItemUI.cs:459-461`).
    #[test]
    fn arrange_swap_slots() {
        let mut i = inv(&[(POTION, 1), (ETHER, 2), (TENT, 3)]);
        arrange_swap(&mut i, 0, 2).unwrap();
        let ids: Vec<u16> = i.slots.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![TENT, ETHER, POTION]);
        assert_eq!(arrange_swap(&mut i, 0, 3), Err(ItemMenuError::BadIndex));
    }

    /// Key items: empty list shows the placeholder (`ItemUI.cs:731`);
    /// opening an item marks it used so the "new" icon goes.
    #[test]
    fn key_items() {
        let mut i = Inventory::default();
        assert_eq!(
            key_item_rows(&i),
            vec![KeyItemRow {
                id: KEY_ITEM_NONE,
                is_new: false
            }]
        );
        assert!(!open_key_item(&mut i, KEY_ITEM_NONE));
        i.add_important(40);
        i.add_important(7);
        assert_eq!(
            key_item_rows(&i),
            vec![
                KeyItemRow {
                    id: 7,
                    is_new: true
                },
                KeyItemRow {
                    id: 40,
                    is_new: true
                },
            ]
        );
        assert!(open_key_item(&mut i, 40));
        assert!(!key_item_rows(&i)[1].is_new);
        assert!(!open_key_item(&mut i, 99));
    }

    /// `DisplayTarget` panels (`ItemUI.cs:782-815`).
    #[test]
    fn target_panels() {
        assert_eq!(TargetDisplay::None.panels(), (true, false));
        assert_eq!(TargetDisplay::Mp.panels(), (true, false));
        assert_eq!(TargetDisplay::Debuffs.panels(), (false, true));
        assert_eq!(TargetDisplay::from_byte(4), Some(TargetDisplay::Buffs));
        assert_eq!(TargetDisplay::from_byte(5), None);
    }
}
