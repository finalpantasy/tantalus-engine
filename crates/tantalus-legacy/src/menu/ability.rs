//! Ability menu rules: ability ids, AP learn state, support-ability
//! capacity, the enable and disable action, and which abilities a character
//! sees (`Global/AbilityUI.cs`, `Memoria/Data/ff9abil.cs`, `Global/ff9feqp.cs`,
//! `Global/ff9play.cs`).

use std::collections::BTreeMap;

use super::{CANNOT_USE_ABILITY_IN_MENU, MenuCharacter, MenuConfig, QUINA, SupportTable};
use crate::shop::{EQUIP_SLOTS, Equipment, ItemDb, NO_ITEM};

// ---------------------------------------------------------------------------
// Ability ids (`ff9abil.cs:316-360`)
// ---------------------------------------------------------------------------

/// `ff9abil.IsAbilityActive` (`ff9abil.cs:316-320`): `id % 256 < 192`.
pub fn is_active(ability_id: i32) -> bool {
    ability_id.rem_euclid(256) < 192
}

/// `ff9abil.IsAbilitySupport` (`ff9abil.cs:322-326`): `id % 256 >= 192`.
pub fn is_support(ability_id: i32) -> bool {
    ability_id.rem_euclid(256) >= 192
}

/// `ff9abil.GetSupportAbilityFromAbilityId` (`ff9abil.cs:337-344`):
/// `pool * 64 + id % 256 - 192`. `None` for an active id (the source returns
/// `SupportAbility.Void`).
pub fn support_index(ability_id: i32) -> Option<i32> {
    is_support(ability_id).then(|| ability_id / 256 * 64 + ability_id % 256 - 192)
}

/// `ff9abil.GetAbilityIdFromSupportAbility` (`ff9abil.cs:354-360`).
pub fn support_ability_id(sa: i32) -> i32 {
    sa / 64 * 256 + sa % 64 + 192
}

/// `ff9abil.GetActiveAbilityFromAbilityId` (`ff9abil.cs:328-335`):
/// `pool * 192 + id % 256`, the `BattleAbilityId` and the index into the
/// action table. `None` for a support id (the source returns `Void`).
pub fn active_index(ability_id: i32) -> Option<i32> {
    is_active(ability_id).then(|| ability_id / 256 * 192 + ability_id % 256)
}

/// `FF9Level_GetCap` without the `MagicStoneStockFormula` option
/// (`Global/ff9level.cs:155-175`): `Gems + level * 4 / 10 + (bonus.cap >> 5)`
/// where `Gems` is the base-stat column. This is the "max capacity from
/// level" of the Ability menu's stone counter.
pub fn max_capacity(base_gems: i32, level: i32, bonus_cap: u16) -> u32 {
    (base_gems + level * 4 / 10 + i32::from(bonus_cap >> 5)) as u32
}

// ---------------------------------------------------------------------------
// AP and learn state (`ff9abil.cs:138-211`)
// ---------------------------------------------------------------------------

/// `ff9abil.FF9Abil_HasAp` (`ff9abil.cs:169-174`).
pub fn has_ap(ch: &MenuCharacter) -> bool {
    ch.learnable.iter().any(|p| p.max_ap > 0)
}

/// `ff9abil.FF9Abil_HasSA` (`ff9abil.cs:176-181`): the preset has a passive.
pub fn has_sa(ch: &MenuCharacter) -> bool {
    ch.learnable.iter().any(|p| is_support(p.id))
}

/// `ff9abil.FF9Abil_GetAp` (`ff9abil.cs:138-144`): -1 when the preset lacks
/// the ability.
pub fn ap_of(ch: &MenuCharacter, ability_id: i32) -> i32 {
    ch.ability_index(ability_id)
        .map_or(-1, |i| ch.pa.get(i).copied().unwrap_or(0))
}

/// `ff9abil.FF9Abil_GetMax` (`ff9abil.cs:161-167`): 0 when absent.
pub fn max_ap_of(ch: &MenuCharacter, ability_id: i32) -> i32 {
    ch.ability_index(ability_id)
        .map_or(0, |i| ch.learnable[i].max_ap)
}

/// `ff9abil.FF9Abil_IsMaster` (`ff9abil.cs:196-201`).
pub fn is_master(ch: &MenuCharacter, ability_id: i32) -> bool {
    let ap = ap_of(ch, ability_id);
    let max = max_ap_of(ch, ability_id);
    ap >= 0 && max >= 0 && ap >= max
}

/// The AP bar numbers (`FF9UIDataTool.DisplayAPBar`,
/// `Assets/Sources/Scripts/UI/Common/FF9UIDataTool.cs:239-261`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ApBar {
    pub cur: i32,
    pub max: i32,
    /// `cur >= max`: the bar is full and the number panel is hidden.
    pub complete: bool,
    /// Number panel visible: false when complete, else the caller's flag.
    pub show_text: bool,
    /// Slider value: 1 when complete, else `cur / max`.
    pub ratio: f32,
}

/// `DisplayAPBar` numbers for an ability the preset holds.
pub fn ap_bar(ch: &MenuCharacter, ability_id: i32, show_text: bool) -> Option<ApBar> {
    ch.ability_index(ability_id)?;
    let cur = ap_of(ch, ability_id);
    let max = max_ap_of(ch, ability_id);
    Some(if cur >= max {
        ApBar {
            cur,
            max,
            complete: true,
            show_text: false,
            ratio: 1.0,
        }
    } else {
        ApBar {
            cur,
            max,
            complete: false,
            show_text,
            ratio: cur as f32 / max as f32,
        }
    })
}

// ---------------------------------------------------------------------------
// Equipment-granted abilities (`AbilityUI.InitialData`, `AbilityUI.cs:1421-1470`)
// ---------------------------------------------------------------------------

/// `equipmentPartInAbilityDict` and `equipmentIdInAbilityDict`: for each
/// ability id on an equipped item, the slot bit mask (bit `i` is slot `i`)
/// and the item in each slot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EquippedAbilities {
    pub parts: BTreeMap<i32, u8>,
    pub items: BTreeMap<i32, [u16; EQUIP_SLOTS]>,
}

impl EquippedAbilities {
    pub fn contains(&self, ability_id: i32) -> bool {
        self.parts.contains_key(&ability_id)
    }
}

/// Builds the maps from the five equipped items. Zero ability slots are
/// skipped (the source stores them under key 0, which no query reads).
pub fn equipped_abilities(db: &ItemDb, equip: &Equipment) -> EquippedAbilities {
    let mut out = EquippedAbilities::default();
    for (slot, &id) in equip.iter().enumerate() {
        if id == NO_ITEM {
            continue;
        }
        let Some(d) = db.get(id) else { continue };
        for &a in d.ability.iter().filter(|a| **a != 0) {
            *out.parts.entry(a).or_insert(0) |= 1 << slot;
            out.items.entry(a).or_insert([NO_ITEM; EQUIP_SLOTS])[slot] = id;
        }
    }
    out
}

/// The active and support lists the menu builds (`InitialData`,
/// `AbilityUI.cs:1423-1451`): ids in preset order, zero ids dropped, and
/// boosted support versions folded into their base entry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AbilityLists {
    pub active: Vec<i32>,
    pub support: Vec<i32>,
}

pub fn ability_lists(ch: &MenuCharacter, t: &SupportTable) -> AbilityLists {
    let mut lists = AbilityLists::default();
    let mut boosted_all: Vec<i32> = Vec::new();
    for p in ch.learnable.iter().filter(|p| p.id != 0) {
        if let Some(sa) = support_index(p.id) {
            lists.support.push(p.id);
            for &b in t.boosted(sa) {
                if ch.learnable.iter().any(|q| support_index(q.id) == Some(b)) {
                    boosted_all.push(b);
                } else {
                    break;
                }
            }
        } else {
            lists.active.push(p.id);
        }
    }
    lists
        .support
        .retain(|id| support_index(*id).is_none_or(|sa| !boosted_all.contains(&sa)));
    lists
}

// ---------------------------------------------------------------------------
// Row states (`AbilityUI.CheckAAType`, `CheckSAType`)
// ---------------------------------------------------------------------------

/// `AbilityUI.AbilityType`: how a row is drawn and whether it can be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbilityType {
    /// Row hidden (not learned, not in the preset, hidden by a feature).
    NoDraw,
    /// Shown gray, cannot be chosen.
    CantSpell,
    /// Can be chosen.
    Enable,
    /// Support ability currently on.
    Selected,
    /// Support ability on and locked (forced, or locked to the equipment).
    CantDisable,
}

/// The `AA_DATA` fields the menu reads: `MP` and `Type` (bit 1: usable in the
/// menu, bit 2: hide the AP text, bit 4: summon; `AbilityUI.cs:1246-1314`,
/// `Assets/Sources/Scripts/UI/Common/FF9UIDataTool.cs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActionInfo {
    pub mp: i32,
    pub kind: u8,
}

/// `AbilityUI.GetMp` (`AbilityUI.cs:1310-1316`): summons cost four times
/// as much while the Garnet summon flag (`gEventGlobal[18]`) is set.
pub fn mp_cost(a: &ActionInfo, garnet_summon_flag: bool) -> i32 {
    if a.kind & 4 != 0 && garnet_summon_flag {
        a.mp << 2
    } else {
        a.mp
    }
}

fn action_for(actions: &[ActionInfo], ability_id: i32) -> Option<ActionInfo> {
    usize::try_from(active_index(ability_id)?)
        .ok()
        .and_then(|i| actions.get(i).copied())
}

/// Inputs that do not belong to the character.
#[derive(Debug, Clone, Copy, Default)]
pub struct AbilityContext {
    pub config: MenuConfig,
    /// `gEventGlobal[18] != 0` (`battle.GARNET_SUMMON_FLAG`).
    pub garnet_summon_flag: bool,
}

/// `AbilityUI.CheckAAType` (`AbilityUI.cs:1246-1269`), with
/// `BattleAbilityHelper.Patch` treated as the identity. An id the action
/// table lacks counts as unusable in the menu.
pub fn check_aa_type(
    ch: &MenuCharacter,
    ctx: &AbilityContext,
    eqp: &EquippedAbilities,
    actions: &[ActionInfo],
    ability_id: i32,
) -> AbilityType {
    let info = action_for(actions, ability_id).unwrap_or_default();
    if !eqp.contains(ability_id) {
        let Some(index) = ch.ability_index(ability_id) else {
            return AbilityType::NoDraw;
        };
        if has_ap(ch) {
            if ctx.config.lock_active && ch.char_id != QUINA {
                return AbilityType::NoDraw;
            }
            let cur = ch.pa.get(index).copied().unwrap_or(0);
            if cur < ch.learnable[index].max_ap {
                return AbilityType::NoDraw;
            }
        }
    }
    if ch.status & CANNOT_USE_ABILITY_IN_MENU != 0
        || info.kind & 1 == 0
        || mp_cost(&info, ctx.garnet_summon_flag) > ch.cur_mp as i32
    {
        AbilityType::CantSpell
    } else {
        AbilityType::Enable
    }
}

/// `AbilityUI.CheckSAType` (`AbilityUI.cs:1271-1308`).
pub fn check_sa_type(
    ch: &MenuCharacter,
    t: &SupportTable,
    ctx: &AbilityContext,
    eqp: &EquippedAbilities,
    ability_id: i32,
) -> AbilityType {
    let Some(sa) = support_index(ability_id) else {
        return AbilityType::NoDraw;
    };
    if ch.hidden.contains(&sa) {
        return AbilityType::NoDraw;
    }
    if ch.banished.contains(&sa) {
        return if ch.forced.contains(&sa) {
            AbilityType::CantDisable
        } else {
            AbilityType::CantSpell
        };
    }
    if ctx.config.lock_support {
        if ch.ability_index(ability_id).is_none() {
            return AbilityType::NoDraw;
        }
        return if eqp.contains(ability_id) || ch.forced.contains(&sa) {
            AbilityType::CantDisable
        } else {
            AbilityType::CantSpell
        };
    }
    if ch.is_enabled(sa) {
        return AbilityType::Selected;
    }
    if !eqp.contains(ability_id) {
        let Some(index) = ch.ability_index(ability_id) else {
            return AbilityType::NoDraw;
        };
        if has_ap(ch) && ch.pa.get(index).copied().unwrap_or(0) < ch.learnable[index].max_ap {
            return AbilityType::NoDraw;
        }
    }
    // The source compares the raw table cost, not the forced-aware cost.
    if i64::from(t.gems(sa)) > i64::from(ch.cur_capa) {
        AbilityType::CantSpell
    } else {
        AbilityType::Enable
    }
}

/// Sub-menu availability (`AbilityUI.InitialData`, `AbilityUI.cs:1471-1472`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubMenuAccess {
    /// `IsSubMenuDisabledByMainMenu(true)` and `(false)`
    /// (`AbilityUI.cs:1544-1549`).
    pub active_disabled_by_main_menu: bool,
    pub support_disabled_by_main_menu: bool,
    /// `gEventGlobal[227] != 0` (`FF9FABIL_EVENT_NOMAGIC`).
    pub no_magic_event: bool,
}

/// `(isAAEnable, isSAEnable)`.
pub fn sub_menu_enabled(
    ch: &MenuCharacter,
    lists: &AbilityLists,
    access: &SubMenuAccess,
) -> (bool, bool) {
    let aa = !access.active_disabled_by_main_menu
        && ch.cur_hp > 0
        && ch.status & CANNOT_USE_ABILITY_IN_MENU == 0
        && !lists.active.is_empty()
        && !access.no_magic_event;
    let sa = !access.support_disabled_by_main_menu && has_sa(ch);
    (aa, sa)
}

/// The row types for one list, in list order, and the index the cursor starts
/// on (`DisplayAA`, `DisplaySA`, `AbilityUI.cs:1011-1118`): the first
/// `Enable` row for active abilities, the first `Enable`, `Selected` or
/// `CantDisable` row for support abilities, else 0.
pub fn first_active_row(types: &[AbilityType], support: bool) -> usize {
    types
        .iter()
        .position(|t| match t {
            AbilityType::Enable => true,
            AbilityType::Selected | AbilityType::CantDisable => support,
            _ => false,
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Capacity (`ff9abil.cs:118-136`, `:259-285`, `:308-314`)
// ---------------------------------------------------------------------------

/// `ff9abil.GetSAGemCostFromPlayer` (`ff9abil.cs:308-314`): forced abilities
/// cost nothing.
pub fn gem_cost(ch: &MenuCharacter, t: &SupportTable, sa: i32) -> i32 {
    if ch.forced.contains(&sa) {
        0
    } else {
        t.gems(sa)
    }
}

/// `ff9abil.FF9Abil_SetEnableSA` (`ff9abil.cs:118-136`). With `gem_cost`
/// set and no support lock, the stones left move by the table cost (not the
/// forced-aware cost). The `player.sa` bit field is not stored; it is the
/// `enabled` set for indexes below 64.
pub fn set_enabled(
    ch: &mut MenuCharacter,
    t: &SupportTable,
    cfg: MenuConfig,
    sa: i32,
    enable: bool,
    with_gem_cost: bool,
) {
    if enable {
        if !ch.enabled.contains(&sa) {
            ch.enabled.push(sa);
        }
    } else {
        ch.enabled.retain(|x| *x != sa);
    }
    if with_gem_cost && !cfg.lock_support {
        let cost = t.gems(sa);
        ch.cur_capa = if enable {
            ch.cur_capa.wrapping_sub(cost as u32)
        } else {
            ch.cur_capa.wrapping_add(cost as u32)
        };
    }
}

/// `ff9abil.CalculateGemsPlayer` (`ff9abil.cs:259-285`): stones left start at
/// the stock; each enabled ability, in set order, is kept when it fits in
/// both the stones left and the stock, and switched off otherwise (its cost
/// is given back, capped at the stock). Skipped under the support lock.
pub fn calculate_gems(ch: &mut MenuCharacter, t: &SupportTable, cfg: MenuConfig) {
    if cfg.lock_support {
        return;
    }
    let equipped = ch.enabled.clone();
    let mut used: i64 = 0;
    ch.cur_capa = ch.max_capa;
    for sa in equipped {
        let cost = i64::from(gem_cost(ch, t, sa));
        if i64::from(ch.cur_capa) >= cost && used + cost <= i64::from(ch.max_capa) {
            used += cost;
            ch.cur_capa = ch.cur_capa.wrapping_sub(cost as u32);
        } else {
            set_enabled(ch, t, cfg, sa, false, false);
            ch.cur_capa = (i64::from(ch.cur_capa) + cost).min(i64::from(ch.max_capa)) as u32;
        }
    }
}

/// The capacity part of `ff9play.FF9Play_Update` (`ff9play.cs:263-334`):
/// `cur.capa = basis.capa`, then `CalculateGemsPlayer`. Stats are computed
/// on demand by [`crate::shop::stats`]; HP and MP feature hooks are not
/// modeled.
pub fn play_update(ch: &mut MenuCharacter, t: &SupportTable, cfg: MenuConfig) {
    ch.cur_capa = ch.basis_capa;
    calculate_gems(ch, t, cfg);
}

/// `ff9feqp.FF9FEqp_UpdateSA` (`ff9feqp.cs:26-60`), run after every
/// equipment change. Does nothing when the preset has no passive. Under the
/// support lock every passive in the preset is switched to "equipped or
/// not". Otherwise an enabled passive that no equipped item grants and that
/// is not fully learned is switched off and its table cost is given back
/// to the stones left (capped at the stock).
pub fn update_sa(ch: &mut MenuCharacter, db: &ItemDb, t: &SupportTable, cfg: MenuConfig) {
    if !has_sa(ch) {
        return;
    }
    let mut equip_sa: Vec<i32> = Vec::new();
    for &id in ch.equip.iter().filter(|&&i| i != NO_ITEM) {
        if let Some(d) = db.get(id) {
            equip_sa.extend(
                d.ability
                    .iter()
                    .filter(|a| **a != 0)
                    .filter_map(|a| support_index(*a)),
            );
        }
    }
    let rows: Vec<(usize, i32, i32)> = ch
        .learnable
        .iter()
        .enumerate()
        .filter_map(|(k, p)| support_index(p.id).map(|sa| (k, sa, p.max_ap)))
        .collect();
    if cfg.lock_support {
        for (_, sa, _) in rows {
            set_enabled(ch, t, cfg, sa, equip_sa.contains(&sa), false);
        }
    } else {
        for (k, sa, max_ap) in rows {
            if ch.is_enabled(sa)
                && !equip_sa.contains(&sa)
                && ch.pa.get(k).copied().unwrap_or(0) < max_ap
            {
                set_enabled(ch, t, cfg, sa, false, false);
                let cost = t.gems(sa) as u32;
                ch.cur_capa = if ch.max_capa.wrapping_sub(ch.cur_capa) >= cost {
                    ch.cur_capa.wrapping_add(cost)
                } else {
                    ch.max_capa
                };
            }
        }
    }
}

/// `ff9feqp.FF9FEqp_UpdatePlayer` (`ff9feqp.cs:19-24`) without the serial
/// number refresh: [`update_sa`] then [`play_update`].
pub fn update_character(ch: &mut MenuCharacter, db: &ItemDb, t: &SupportTable, cfg: MenuConfig) {
    update_sa(ch, db, t, cfg);
    play_update(ch, t, cfg);
}

// ---------------------------------------------------------------------------
// Boosted versions (`ff9abil.cs:213-306`)
// ---------------------------------------------------------------------------

/// `ff9abil.GetBaseAbilityFromBoostedAbility` (`ff9abil.cs:218-224`).
pub fn base_of_boosted(t: &SupportTable, sa: i32) -> i32 {
    t.rows
        .iter()
        .position(|r| r.boosted.contains(&sa))
        .map_or(sa, |i| i as i32)
}

/// `ff9abil.GetHierarchyFromAnySA` (`ff9abil.cs:226-235`): base first, then
/// its boosted versions.
pub fn hierarchy(t: &SupportTable, sa: i32) -> Vec<i32> {
    let base = base_of_boosted(t, sa);
    let mut v = vec![base];
    v.extend_from_slice(t.boosted(base));
    v
}

/// `ff9abil.GetNonForcedSAInHierarchy` (`ff9abil.cs:254-257`).
pub fn non_forced_in_hierarchy(ch: &MenuCharacter, t: &SupportTable, sa: i32) -> Vec<i32> {
    hierarchy(t, sa)
        .into_iter()
        .filter(|s| !ch.forced.contains(s))
        .collect()
}

/// `ff9abil.GetBoostedAbilityMaxLevel` (`ff9abil.cs:287-296`): how many
/// boosted versions the preset holds, counted from the first.
pub fn boosted_max_level(ch: &MenuCharacter, t: &SupportTable, base: i32) -> i32 {
    if ch.learnable.is_empty() {
        return 0;
    }
    let boosted = t.boosted(base);
    for (level, b) in boosted.iter().enumerate() {
        if !ch.learnable.iter().any(|p| p.id == support_ability_id(*b)) {
            return level as i32;
        }
    }
    boosted.len() as i32
}

/// `ff9abil.GetBoostedAbilityLevel` (`ff9abil.cs:298-306`): how many boosted
/// versions are on, counted from the first.
pub fn boosted_level(ch: &MenuCharacter, t: &SupportTable, base: i32) -> i32 {
    let boosted = t.boosted(base);
    for (level, b) in boosted.iter().enumerate() {
        if !ch.is_enabled(*b) {
            return level as i32;
        }
    }
    boosted.len() as i32
}

/// The stones and level a support row shows (`DisplaySADetail`,
/// `AbilityUI.cs:1156-1177`): for an `on` row with boosted versions, the
/// summed cost of the base and the boosted versions that are on, and the
/// version whose name is shown.
pub fn support_row_display(
    ch: &MenuCharacter,
    t: &SupportTable,
    sa: i32,
    ty: AbilityType,
) -> (i32, i32) {
    let mut shown = sa;
    let mut cost = gem_cost(ch, t, sa);
    if matches!(ty, AbilityType::Selected | AbilityType::CantDisable) {
        let max = boosted_max_level(ch, t, sa);
        if max > 0 {
            let level = max.min(boosted_level(ch, t, sa));
            let list = t.boosted(sa);
            for b in list.iter().take(level as usize) {
                cost += gem_cost(ch, t, *b);
            }
            if level > 0 {
                shown = list[(level - 1) as usize];
            }
        }
    }
    (shown, cost)
}

// ---------------------------------------------------------------------------
// Enable and disable (`AbilityUI.OnKeyConfirm`, `AbilityUI.cs:411-478`)
// ---------------------------------------------------------------------------

/// Why a confirm on a support row did nothing (the source plays the refusal
/// sound).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportRefusal {
    /// The row is hidden, grayed or locked (`CantSpell`, `CantDisable`,
    /// `NoDraw`), or the id is not a support ability.
    NotAvailable(AbilityType),
    /// Every ability in the hierarchy is forced (`AbilityUI.cs:437-441`).
    AllForced,
}

/// What the confirm did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportToggle {
    /// An unselected ability was switched on and its cost taken from the
    /// stones left.
    Enabled,
    /// A selected ability was switched off, or its next boosted version was
    /// switched on, and the stones and the stats were recomputed.
    Disabled,
    BoostedUp,
}

/// Confirm on a support row. `Enable`: switch on, take the cost, run
/// `FF9Play_Update`. `Selected`: with boosted versions, switch on the next
/// one when it fits, otherwise switch off every boosted version and the base;
/// without them, switch off the base. Costs are given back at the forced-aware
/// price (`GetSAGemCostFromPlayer`). `CantSpell`, `CantDisable` and `NoDraw`
/// refuse.
pub fn toggle_support(
    ch: &mut MenuCharacter,
    t: &SupportTable,
    ctx: &AbilityContext,
    eqp: &EquippedAbilities,
    ability_id: i32,
) -> Result<SupportToggle, SupportRefusal> {
    let cfg = ctx.config;
    let ty = check_sa_type(ch, t, ctx, eqp, ability_id);
    let Some(sa) = support_index(ability_id) else {
        return Err(SupportRefusal::NotAvailable(ty));
    };
    match ty {
        AbilityType::Enable => {
            set_enabled(ch, t, cfg, sa, true, false);
            ch.cur_capa = ch.cur_capa.wrapping_sub(gem_cost(ch, t, sa) as u32);
            play_update(ch, t, cfg);
            Ok(SupportToggle::Enabled)
        }
        AbilityType::Selected => {
            if non_forced_in_hierarchy(ch, t, sa).is_empty() {
                return Err(SupportRefusal::AllForced);
            }
            let max = boosted_max_level(ch, t, sa);
            let mut result = SupportToggle::Disabled;
            if max > 0 {
                let level = max.min(boosted_level(ch, t, sa));
                let list = t.boosted(sa).to_vec();
                let mut enable_next = level < max;
                if enable_next {
                    let next = list[level as usize];
                    if ch.banished.contains(&next) {
                        enable_next = false;
                    } else {
                        enable_next = check_sa_type(ch, t, ctx, eqp, support_ability_id(next))
                            == AbilityType::Enable;
                        if enable_next {
                            set_enabled(ch, t, cfg, next, true, false);
                            ch.cur_capa = ch.cur_capa.wrapping_sub(gem_cost(ch, t, next) as u32);
                            result = SupportToggle::BoostedUp;
                        }
                    }
                }
                if !enable_next {
                    for b in list {
                        if ch.is_enabled(b) {
                            set_enabled(ch, t, cfg, b, false, false);
                            ch.cur_capa = ch.cur_capa.wrapping_add(gem_cost(ch, t, b) as u32);
                        }
                    }
                    set_enabled(ch, t, cfg, sa, false, false);
                    ch.cur_capa = ch.cur_capa.wrapping_add(gem_cost(ch, t, sa) as u32);
                }
            } else {
                set_enabled(ch, t, cfg, sa, false, false);
                ch.cur_capa = ch.cur_capa.wrapping_add(gem_cost(ch, t, sa) as u32);
            }
            play_update(ch, t, cfg);
            Ok(result)
        }
        other => Err(SupportRefusal::NotAvailable(other)),
    }
}

/// The secondary key on a support row (`AbilityUI.OnKeySpecial`,
/// `AbilityUI.cs:626-715`). `Enable`: switch on the base and then every
/// boosted version in order while each one is `Enable` (or forced and
/// already on); the first that is not stops the run. `Selected`: step down
/// one boosted level (the highest version that is not forced, else the
/// base) and switch only that one off; when the base itself is forced and
/// every lower version is gone, the source switches boosted versions back on
/// first. The source loop has no bound; this port stops after 64 passes.
pub fn toggle_support_special(
    ch: &mut MenuCharacter,
    t: &SupportTable,
    ctx: &AbilityContext,
    eqp: &EquippedAbilities,
    ability_id: i32,
) -> Result<SupportToggle, SupportRefusal> {
    let cfg = ctx.config;
    let ty = check_sa_type(ch, t, ctx, eqp, ability_id);
    let Some(base) = support_index(ability_id) else {
        return Err(SupportRefusal::NotAvailable(ty));
    };
    let eligible = |ch: &MenuCharacter, b: i32| {
        let bt = check_sa_type(ch, t, ctx, eqp, support_ability_id(b));
        bt == AbilityType::Enable || (ch.forced.contains(&b) && bt == AbilityType::Selected)
    };
    match ty {
        AbilityType::Enable => {
            set_enabled(ch, t, cfg, base, true, false);
            ch.cur_capa = ch.cur_capa.wrapping_sub(gem_cost(ch, t, base) as u32);
            if boosted_max_level(ch, t, base) > 0 {
                for b in t.boosted(base).to_vec() {
                    if eligible(ch, b) {
                        set_enabled(ch, t, cfg, b, true, false);
                        ch.cur_capa = ch.cur_capa.wrapping_sub(gem_cost(ch, t, b) as u32);
                    } else {
                        break;
                    }
                }
            }
            play_update(ch, t, cfg);
            Ok(SupportToggle::Enabled)
        }
        AbilityType::Selected => {
            if non_forced_in_hierarchy(ch, t, base).is_empty() {
                return Err(SupportRefusal::AllForced);
            }
            let max = boosted_max_level(ch, t, base);
            let mut target = base;
            if max > 0 {
                let mut level = max.min(boosted_level(ch, t, base));
                let list = t.boosted(base).to_vec();
                let mut previous_available = false;
                let mut passes = 0;
                while !previous_available && passes < 64 {
                    passes += 1;
                    if level > 0 && !ch.forced.contains(&list[(level - 1) as usize]) {
                        target = list[(level - 1) as usize];
                        previous_available = true;
                    }
                    level -= 1;
                    if ch.forced.contains(&target) && level < 0 {
                        for b in &list {
                            if eligible(ch, *b) {
                                set_enabled(ch, t, cfg, *b, true, false);
                                ch.cur_capa = ch.cur_capa.wrapping_sub(gem_cost(ch, t, *b) as u32);
                            } else {
                                previous_available = true;
                            }
                        }
                    } else {
                        previous_available = true;
                    }
                }
            }
            set_enabled(ch, t, cfg, target, false, false);
            ch.cur_capa = ch.cur_capa.wrapping_add(gem_cost(ch, t, target) as u32);
            play_update(ch, t, cfg);
            Ok(SupportToggle::Disabled)
        }
        other => Err(SupportRefusal::NotAvailable(other)),
    }
}

/// After a successful field cast (`AbilityUI.OnKeyConfirm`,
/// `AbilityUI.cs:527-531`): the caster pays the MP unless the "HP and MP
/// full" cheat is on. The cast effect itself (`SFieldCalculator`) is not
/// modeled. Returns the MP paid and whether the caster can still afford the
/// same ability (when not, the source returns the cursor to the list).
pub fn spend_mp(ch: &mut MenuCharacter, cost: i32, hp_mp_full: bool) -> (i32, bool) {
    if !hp_mp_full {
        ch.cur_mp = ch.cur_mp.wrapping_sub(cost as u32);
    }
    (cost, i64::from(ch.cur_mp) >= i64::from(cost))
}

// ---------------------------------------------------------------------------
// Item ability display (`EquipUI.DisplayAbilityState`, `EquipUI.cs:720-768`)
// ---------------------------------------------------------------------------

/// The stone sprite kind of `DisplayAbilityState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stone {
    /// `ability_stone`: active ability in the preset.
    Active,
    /// `ability_stone_null`.
    ActiveNull,
    /// `skill_stone_on`: support ability in the preset and switched on.
    SupportOn,
    /// `skill_stone_off`: in the preset, switched off.
    SupportOff,
    /// `skill_stone_null`.
    SupportNull,
}

/// One ability of an item as the Equip menu shows it for a character.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemAbilityState {
    pub ability: i32,
    pub support: bool,
    /// The preset holds the ability (`FF9Abil_GetIndex >= 0`) and the
    /// character has an AP table.
    pub in_preset: bool,
    pub stone: Stone,
    /// Name drawn white (`in_preset`) or gray.
    pub white: bool,
    pub ap: Option<ApBar>,
}

/// `DisplayAbilityState` for every non-zero ability of `item`, in item order.
pub fn item_ability_states(
    ch: &MenuCharacter,
    db: &ItemDb,
    actions: &[ActionInfo],
    item: u16,
) -> Vec<ItemAbilityState> {
    let Some(d) = db.get(item) else {
        return Vec::new();
    };
    d.ability
        .iter()
        .copied()
        .filter(|a| *a != 0)
        .map(|a| {
            let support = is_support(a);
            if !has_ap(ch) {
                return ItemAbilityState {
                    ability: a,
                    support,
                    in_preset: false,
                    stone: if support {
                        Stone::SupportNull
                    } else {
                        Stone::ActiveNull
                    },
                    white: false,
                    ap: None,
                };
            }
            let in_preset = ch.ability_index(a).is_some();
            let stone = match (in_preset, support) {
                (true, false) => Stone::Active,
                (true, true) => {
                    if support_index(a).is_some_and(|sa| ch.is_enabled(sa)) {
                        Stone::SupportOn
                    } else {
                        Stone::SupportOff
                    }
                }
                (false, false) => Stone::ActiveNull,
                (false, true) => Stone::SupportNull,
            };
            let show_text = support || action_for(actions, a).is_none_or(|x| x.kind & 2 == 0);
            ItemAbilityState {
                ability: a,
                support,
                in_preset,
                stone,
                white: in_preset,
                ap: if in_preset {
                    ap_bar(ch, a, show_text)
                } else {
                    None
                },
            }
        })
        .collect()
}

/// `(abilityCount + 2) / 3` (`EquipUI.DisplayEquiptmentInfo`,
/// `EquipUI.cs:679`): the item ability panel pages through three at a time
/// when there are more than three.
pub fn ability_page_count(db: &ItemDb, item: u16) -> usize {
    let n = db
        .get(item)
        .map_or(0, |d| d.ability.iter().filter(|a| **a != 0).count());
    n.div_ceil(3)
}
