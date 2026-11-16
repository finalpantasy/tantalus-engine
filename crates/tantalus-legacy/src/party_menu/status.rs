//! Status screen derivations (`Global/StatusUI.cs`): the nine parameters,
//! EXP and next-level numbers, the Trance gauge test, party browsing and the
//! ability page state.
//!
//! The parameters are the numbers `FF9Play_Update` leaves in `elem` and
//! `defence` (`Global/ff9play.cs:263-312`), which [`crate::shop::stats`]
//! already computes; this module reads them the way `DisplayAllCharacterInfo`
//! does (`StatusUI.cs:276-310`).

use crate::battle::rewards::{LEVEL_COUNT, LevelTables, PaEntry};
use crate::shop::{Equipment, ItemDb, stats};

/// `ff9item.GetItemWeapon(equip[0]).Ref.Power` when the weapon slot holds no
/// item with a weapon row: `GetItemWeapon` logs an error and returns
/// `ff9weap.WeaponData[0]` (`Memoria/Data/ff9item.2.cs:504-513`), the stock
/// Hammer, power 12
/// (`Memoria.Patcher/StreamingAssets/Data/Items/Weapons.csv:10`). Item 255
/// (`NoItem`) has `WeaponId -1` (`Items.csv:265`). A modded `Weapons.csv`
/// would change this number.
pub const FALLBACK_WEAPON_POWER: i32 = 12;

/// `gEventGlobal` index that turns the Trance gauge on (`StatusUI.cs:291`).
pub const EVENT_TRANCE_GAUGE: usize = 16;
/// `category & 16` marks a character with no Trance (`StatusUI.cs:291`).
pub const CATEGORY_NO_TRANCE: u8 = 16;

/// Ability pages (`StatusUI.OnKeyConfirm`, `StatusUI.cs:111-142`: the
/// page counter runs to 6).
pub const ABILITY_PAGES: u8 = 6;
/// Ability rows per page (`StatusUI.cs:411`, `childIndex < 8`).
pub const ABILITIES_PER_PAGE: usize = 8;

/// What the screen reads from one party member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusInput {
    /// `basis.dex`, `str`, `mgc`, `wpr` (Speed, Strength, Magic, Spirit).
    pub basis: [i32; 4],
    /// Equipped item ids: weapon, helmet, armlet, armor, accessory; 255 empty.
    pub equip: Equipment,
    pub level: u8,
    pub exp: u32,
    /// `PLAYER.category`.
    pub category: u8,
    /// `PLAYER.trance`.
    pub trance: u8,
}

/// The values the Status screen prints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatusView {
    pub speed: i32,
    pub strength: i32,
    pub magic: i32,
    pub spirit: i32,
    pub attack: i32,
    pub defense: i32,
    pub evade: i32,
    pub magic_defense: i32,
    pub magic_evade: i32,
    pub exp: u32,
    pub next_level: u32,
    /// `Some(fill)` when the Trance gauge shows: `trance / 256f`
    /// (`StatusUI.cs:294`), so a full byte reads 255/256, never 1.
    pub trance_fill: Option<f32>,
}

/// EXP needed for the next level (`StatusUI.cs:290-298`,
/// `MainMenuUI.cs:461-464`): the threshold of `level + 1` minus current EXP,
/// computed in `UInt32` (so it wraps if EXP is somehow past the threshold).
/// At level 99 (`LEVEL_COUNT`) the threshold is the EXP itself, so 0.
pub fn exp_to_next(levels: &LevelTables, level: u8, exp: u32) -> u32 {
    let threshold = if usize::from(level) < LEVEL_COUNT {
        levels.exp_for_next(level)
    } else {
        exp
    };
    threshold.wrapping_sub(exp)
}

/// `DisplayAllCharacterInfo` (`StatusUI.cs:276-310`).
///
/// Not modeled: support-ability feature hooks that rewrite `elem` and
/// `defence` inside `FF9Play_Update` (`FF9Play_SAFeature_Update`), same as
/// [`crate::shop::stats`].
pub fn status_view(
    db: &ItemDb,
    levels: &LevelTables,
    input: &StatusInput,
    event_trance_flag: u8,
) -> StatusView {
    let s = stats(db, input.basis, &input.equip);
    let weapon = input.equip[0];
    let attack = db
        .get(weapon)
        .and_then(|d| d.weapon_power)
        .unwrap_or(FALLBACK_WEAPON_POWER);
    let trance_fill = (event_trance_flag != 0 && input.category & CATEGORY_NO_TRANCE == 0)
        .then(|| f32::from(input.trance) / 256.0);
    StatusView {
        speed: s.dex,
        strength: s.str_,
        magic: s.mgc,
        spirit: s.wpr,
        attack,
        defense: s.p_def,
        evade: s.p_ev,
        magic_defense: s.m_def,
        magic_evade: s.m_ev,
        exp: input.exp,
        next_level: exp_to_next(levels, input.level, input.exp),
        trance_fill,
    }
}

/// `ff9play.FF9Play_GetPrev` (`ff9play.cs:167-174`): the previous non-empty
/// party slot, wrapping 0 to 3. Returns `id` itself when it is the only
/// member. `None` when the party is empty (the source would loop forever).
pub fn prev_member(present: &[bool; 4], id: usize) -> Option<usize> {
    let mut i = id;
    for _ in 0..4 {
        i = if i == 0 { 3 } else { i - 1 };
        if present[i] {
            return Some(i);
        }
    }
    None
}

/// `ff9play.FF9Play_GetNext` (`ff9play.cs:176-183`).
pub fn next_member(present: &[bool; 4], id: usize) -> Option<usize> {
    let mut i = id;
    for _ in 0..4 {
        i = if i >= 3 { 0 } else { i + 1 };
        if present[i] {
            return Some(i);
        }
    }
    None
}

/// One ability row on the Status ability pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbilityRow {
    /// Index into `pa` / the preset's learnable list.
    pub index: usize,
    pub ability_id: i32,
    /// Learned AP so far (`pa[index]`).
    pub ap: i32,
    /// AP needed (`CharacterAbility.Ap`).
    pub max_ap: i32,
}

/// `DrawAbilityInfo` visibility (`StatusUI.cs:340-358`): a row is hidden
/// when the index is past `pa`, when the character earns AP (any learnable
/// row has `max_ap > 0`, `ff9abil.FF9Abil_HasAp`) and this one has none yet,
/// or when its ability id is 0. A preset where no
/// learnable row has AP lists every non-zero ability. Active versus support icons
/// and the enabled stone need the ability tables and are not decided here.
pub fn visible_ability_rows(learnable: &[PaEntry], pa: &[i32]) -> Vec<AbilityRow> {
    let has_ap = learnable.iter().any(|p| p.max_ap > 0);
    let mut out = Vec::new();
    for (index, &ap) in pa.iter().enumerate() {
        if has_ap && ap == 0 {
            continue;
        }
        let Some(entry) = learnable.get(index) else {
            continue;
        };
        if entry.id == 0 {
            continue;
        }
        out.push(AbilityRow {
            index,
            ability_id: entry.id,
            ap,
            max_ap: entry.max_ap,
        });
    }
    out
}

/// What a confirm press on the Status screen did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelStep {
    /// Page `n` (0-based) slid in.
    Opened(u8),
    /// All pages slid out.
    ClosedAll,
}

/// What a cancel press did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelStep {
    /// Pages were open: all slid out, still on the screen.
    ClosedPages,
    /// No page open: leave to the main menu.
    Exit,
}

/// The ability pages opened so far (`_currentAbilityPanelAmount`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AbilityPanel {
    pub opened: u8,
}

impl AbilityPanel {
    /// `OnKeyConfirm` (`StatusUI.cs:111-142`): each press opens the next
    /// page; after the sixth the next press closes all.
    pub fn confirm(&mut self) -> PanelStep {
        if self.opened < ABILITY_PAGES {
            let page = self.opened;
            self.opened += 1;
            PanelStep::Opened(page)
        } else {
            self.opened = 0;
            PanelStep::ClosedAll
        }
    }

    /// `OnKeyCancel` (`StatusUI.cs:148-170`).
    pub fn cancel(&mut self) -> CancelStep {
        if self.opened > 0 {
            self.opened = 0;
            CancelStep::ClosedPages
        } else {
            CancelStep::Exit
        }
    }

    /// `DisplayPlayerArrow` (`StatusUI.cs:265-272`): the character arrows
    /// show only with no page open and more than one member.
    pub fn arrows_visible(&self, member_count: usize) -> bool {
        self.opened == 0 && member_count > 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shop::data::item_type;
    use crate::shop::{ArmorData, ItemData, NO_ITEM, StatBonus};
    use std::collections::BTreeMap;

    const DAGGER: u16 = 1;
    const WRIST: u16 = 88;
    const HELM: u16 = 100;

    fn db() -> ItemDb {
        let base = |kind: u8| ItemData {
            price: 0,
            selling_price: 0,
            equip: 0,
            kind,
            sort: 0.0,
            eq_lv: 0.0,
            bonus: 0,
            ability: vec![],
            weapon_power: None,
            armor: None,
        };
        let mut items = BTreeMap::new();
        let mut d = base(item_type::WEAPON);
        d.weapon_power = Some(25);
        d.bonus = 1; // +3 strength
        items.insert(DAGGER, d);
        let mut d = base(item_type::ARMLET);
        d.armor = Some(ArmorData {
            p_def: 4,
            p_ev: 5,
            m_def: 6,
            m_ev: 7,
        });
        d.bonus = 2; // +60 magic, to test the 99 cap
        items.insert(WRIST, d);
        let mut d = base(item_type::HELMET);
        d.armor = Some(ArmorData {
            p_def: 990,
            p_ev: 0,
            m_def: 0,
            m_ev: 0,
        });
        items.insert(HELM, d);
        items.insert(NO_ITEM, base(0));
        ItemDb {
            items,
            bonuses: vec![
                StatBonus::default(),
                StatBonus {
                    dex: 0,
                    str_: 3,
                    mgc: 0,
                    wpr: 0,
                },
                StatBonus {
                    dex: 0,
                    str_: 0,
                    mgc: 60,
                    wpr: 0,
                },
            ],
        }
    }

    fn levels() -> LevelTables {
        // Row n is the threshold of level n + 1 (`Leveling.csv:7-8`: row 0
        // is 0, row 1 is 16); only rows 1 to 3 matter here.
        let mut exp = vec![0u32; 100];
        exp[1] = 16;
        exp[2] = 40;
        exp[3] = 90;
        LevelTables::new(exp, vec![0; 100], vec![0; 100]).unwrap()
    }

    /// `StatusUI.cs:276-310` by hand. Basis (spd 20, str 25, mgc 90, spr 30).
    /// Dagger: attack 25, +3 str. Wrist: defence 4/5/6/7, +60 mgc.
    /// Helmet: defence 990 (cap 999 not reached alone). Totals: str 28,
    /// mgc 150 capped to 99, defence 994 (4 + 990), spd and spr unchanged.
    #[test]
    fn parameters() {
        let input = StatusInput {
            basis: [20, 25, 90, 30],
            equip: [DAGGER, HELM, WRIST, NO_ITEM, NO_ITEM],
            level: 2,
            exp: 25,
            category: 0,
            trance: 128,
        };
        let v = status_view(&db(), &levels(), &input, 0);
        assert_eq!((v.speed, v.strength, v.magic, v.spirit), (20, 28, 99, 30));
        assert_eq!(v.attack, 25);
        assert_eq!(
            (v.defense, v.evade, v.magic_defense, v.magic_evade),
            (994, 5, 6, 7)
        );
        // Level 2: threshold of level 3 is row 2 = 40, minus 25 = 15.
        assert_eq!((v.exp, v.next_level), (25, 15));
        assert_eq!(v.trance_fill, None);
    }

    /// Defence caps at 999 after summing (`ff9play.cs:300-307`).
    #[test]
    fn defence_cap() {
        let mut d = db();
        d.items.get_mut(&WRIST).unwrap().armor = Some(ArmorData {
            p_def: 50,
            p_ev: 0,
            m_def: 0,
            m_ev: 0,
        });
        let input = StatusInput {
            basis: [1, 1, 1, 1],
            equip: [DAGGER, HELM, WRIST, 255, 255],
            level: 1,
            exp: 0,
            category: 0,
            trance: 0,
        };
        let v = status_view(&d, &levels(), &input, 0);
        assert_eq!(v.defense, 999);
    }

    /// With an empty weapon slot the source reads weapon row 0 (the Hammer,
    /// power 12) instead of showing 0.
    #[test]
    fn empty_weapon_shows_fallback_row() {
        let input = StatusInput {
            basis: [1, 1, 1, 1],
            equip: [255; 5],
            level: 1,
            exp: 0,
            category: 0,
            trance: 0,
        };
        let v = status_view(&db(), &levels(), &input, 0);
        assert_eq!(v.attack, FALLBACK_WEAPON_POWER);
    }

    /// Trance gauge: shown only when `gEventGlobal[16] != 0` and category bit
    /// 16 is clear; fill is `trance / 256`.
    #[test]
    fn trance_gauge() {
        let mut input = StatusInput {
            basis: [1, 1, 1, 1],
            equip: [255; 5],
            level: 1,
            exp: 0,
            category: 0,
            trance: 64,
        };
        assert_eq!(
            status_view(&db(), &levels(), &input, 1).trance_fill,
            Some(0.25)
        );
        input.trance = 255;
        assert_eq!(
            status_view(&db(), &levels(), &input, 7).trance_fill,
            Some(255.0 / 256.0)
        );
        input.category = 16;
        assert_eq!(status_view(&db(), &levels(), &input, 1).trance_fill, None);
        input.category = 1 | 2 | 4 | 8;
        assert!(
            status_view(&db(), &levels(), &input, 1)
                .trance_fill
                .is_some()
        );
        assert!(
            status_view(&db(), &levels(), &input, 0)
                .trance_fill
                .is_none()
        );
    }

    /// Level 1 needs row 1 = 16; at level 99 the next-level number is 0;
    /// EXP past the threshold wraps like `UInt32` subtraction.
    #[test]
    fn exp_numbers() {
        let l = levels();
        assert_eq!(exp_to_next(&l, 1, 0), 16);
        assert_eq!(exp_to_next(&l, 1, 15), 1);
        assert_eq!(exp_to_next(&l, 99, 1_000_000), 0);
        assert_eq!(exp_to_next(&l, 1, 17), u32::MAX);
    }

    /// `FF9Play_GetPrev` / `GetNext` skip empty slots and wrap.
    #[test]
    fn member_browsing() {
        let present = [true, false, true, false];
        assert_eq!(next_member(&present, 0), Some(2));
        assert_eq!(next_member(&present, 2), Some(0));
        assert_eq!(prev_member(&present, 0), Some(2));
        assert_eq!(prev_member(&present, 2), Some(0));
        assert_eq!(next_member(&[false, true, false, false], 1), Some(1));
        assert_eq!(prev_member(&[false; 4], 0), None);
    }

    /// Ability rows (`StatusUI.cs:340-358`): a character that earns AP shows
    /// only abilities with AP above 0 and a non-zero id; one that does not
    /// shows every non-zero row.
    #[test]
    fn ability_rows() {
        let learn = [
            PaEntry { id: 10, max_ap: 20 },
            PaEntry { id: 11, max_ap: 30 },
            PaEntry { id: 0, max_ap: 0 },
            PaEntry { id: 12, max_ap: 40 },
        ];
        let rows = visible_ability_rows(&learn, &[5, 0, 9, 40]);
        assert_eq!(
            rows.iter()
                .map(|r| (r.index, r.ability_id, r.ap))
                .collect::<Vec<_>>(),
            vec![(0, 10, 5), (3, 12, 40)]
        );
        assert_eq!(rows[1].max_ap, 40);
        // No AP anywhere: zero progress rows still show, id 0 does not.
        let free = [PaEntry { id: 7, max_ap: 0 }, PaEntry { id: 0, max_ap: 0 }];
        let rows = visible_ability_rows(&free, &[0, 0]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ability_id, 7);
        // `pa` longer than the list: extra indices are skipped.
        assert_eq!(visible_ability_rows(&free, &[0, 0, 0]).len(), 1);
    }

    /// Ability pages: six presses open six pages, the seventh closes all;
    /// cancel closes pages first, then leaves; arrows only with no page open.
    #[test]
    fn ability_panel_flow() {
        let mut p = AbilityPanel::default();
        assert!(p.arrows_visible(2));
        assert!(!p.arrows_visible(1));
        for i in 0..6 {
            assert_eq!(p.confirm(), PanelStep::Opened(i));
            assert!(!p.arrows_visible(4));
        }
        assert_eq!(p.confirm(), PanelStep::ClosedAll);
        assert_eq!(p.opened, 0);
        assert_eq!(p.confirm(), PanelStep::Opened(0));
        assert_eq!(p.cancel(), CancelStep::ClosedPages);
        assert_eq!(p.opened, 0);
        assert_eq!(p.cancel(), CancelStep::Exit);
    }
}
