//! Battle results (milestone B8): bonus accumulation, EXP/AP/gil/item
//! distribution, level-up, ability learning, flee bookkeeping.
//!
//! Pure and deterministic: item and card draws go through [`BattleRandom`],
//! nothing reads a clock or a file. Source is Memoria pin `c716e1fa`; paths are
//! relative to `Assembly-CSharp/`.
//!
//! Flow in the source:
//!
//! 1. Each enemy death runs `btl_sys.SetBonus` (`Global/btl_mot.cs`-adjacent
//!    `FF9/btl_mot.cs:344`, `FF9/btl_sys.cs:215-258`), adding gil, EXP and item
//!    or card draws into the global `BONUS` (`Global/BONUS.cs`).
//!    [`BattleBonus::set_bonus`].
//! 2. The result screen splits and applies it
//!    (`Global/battle/Result/BattleResultUI.cs`). [`BattleReward::from_bonus`]
//!    computes the split; [`BattleReward::apply`] applies it once.
//! 3. A flee clears the bonus and may cost gil (`Global/battle/battle.cs:456-473`,
//!    `Global/battle/BattleHUD/BattleHUD.Public.cs:734-760`). [`settle_escape`].
//!
//! Not modeled (listed in `audit/playable-milestones.md` row B8): support
//! ability features (`TriggerOnBattleResult` "RewardAll", "RewardSingle" and
//! "BattleEnd", `TriggerOnEnable`, `CalculateGemsPlayer`, `GetEnabledSA`),
//! Tetra Master card storage, animation and sound timing, the UI tick
//! pacing (only its arithmetic effect is kept), and the Memoria
//! `*StatFormula` / `MagicStoneStockFormula` overrides (empty by default).

use super::flee::flee_gil_loss;
use super::rng::BattleRandom;

/// `RegularItem.NoItem` (`Memoria/Data/Battle/RegularItem.cs:5`).
pub const NO_ITEM: u32 = 255;
/// `TetraMasterCardId.NONE` (`Memoria/Data/TetraMaster/TetraMasterCardId.cs:107`).
pub const NO_CARD: u32 = 255;
/// EXP ceiling: `play.exp < 9999999u` (`BattleResultUI.cs:318`, `:461-465`).
pub const EXP_CAP: u32 = 9_999_999;
/// Gil ceiling: `9999999u < party.gil + gilAdd` (`BattleResultUI.cs:735-741`).
pub const GIL_CAP: u32 = 9_999_999;
/// `FF9ITEM_COUNT_MAX` (`Memoria/Data/ff9item.2.cs:40`).
pub const ITEM_COUNT_MAX: u32 = 99;
/// `BattleResultUI.ItemMax` (`BattleResultUI.cs:854`).
pub const RESULT_ITEM_MAX: usize = 6;
/// `ff9level.LEVEL_COUNT` (`Global/ff9level.cs:13`).
pub const LEVEL_COUNT: usize = 99;
/// `FF9PLAY_HP_MAX`, `FF9PLAY_MP_MAX` (`Global/ff9play.cs:44-45`).
pub const HP_MAX: u32 = 9999;
pub const MP_MAX: u32 = 999;
/// `FF9PLAY_STAT_MAX` (dex, str, mgc, wpr) (`Global/ff9play.cs:53`).
pub const STAT_MAX: [u8; 4] = [50, 99, 99, 50];

// ---------------------------------------------------------------------------
// Enemy side: SetBonus
// ---------------------------------------------------------------------------

/// Reward fields of one enemy slot (`ENEMY` bonus fields set at
/// `Global/btl_init.cs:167-175`). `type_id` stands for the `et` enemy-type
/// pointer compared in `SetBonus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnemyRewardData {
    pub type_id: u32,
    /// `btl_id != 0` (the slot holds a unit).
    pub present: bool,
    pub gil: u32,
    pub exp: u32,
    /// `WinItems`; [`NO_ITEM`] marks an empty slot. Slot 0 is the common drop.
    pub items: [u32; 4],
    /// `WinItemRates`, compared as `random8() < rate`.
    pub item_rates: [u16; 4],
    /// `WinCard`, [`NO_CARD`] for none.
    pub card: u32,
    pub card_rate: u16,
    /// `BTL_DATA.bonus_given`. Read by `SetBonus`; the pinned source never
    /// writes it `true`, so this port never does either.
    pub bonus_given: bool,
}

/// Global `BONUS` (`Global/BONUS.cs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleBonus {
    pub gil: i32,
    pub exp: u32,
    pub ap: u16,
    /// Bit `i` set when party line `i` took part (`btl_init.cs:365`).
    pub member_flag: u8,
    /// `btl_scene.Info.AfterEvent` (`battle.cs:72`).
    pub event: bool,
    pub items: Vec<u32>,
    pub card: u32,
}

impl BattleBonus {
    /// State after `InitBattleSystem` + `SetupBattleScene`: gil/EXP/items
    /// cleared (`btl_sys.cs:42-50`), AP set to the pattern's `AP`
    /// (`btlseq.cs:472`, `(UInt16)` cast).
    pub fn new(pattern_ap: u32, member_flag: u8, event: bool) -> Self {
        Self {
            gil: 0,
            exp: 0,
            ap: pattern_ap as u16,
            member_flag,
            event,
            items: Vec::new(),
            card: NO_CARD,
        }
    }

    /// `btl_sys.SetBonus(BTL_DATA)` + `SetBonus(ENEMY)`
    /// (`FF9/btl_sys.cs:215-258`) for `enemies[index]`. Returns false (no
    /// change, no draw) when the unit was already paid (`bonus_given`).
    ///
    /// Draw order: slot 3 (only when it holds an item), else slot 2, else
    /// slot 1 (`else if` chain, a failed roll falls to the next slot), then
    /// slot 0 always independently, then the card. Each draw is one
    /// `random8`. A slot 1-3 hit blanks the same slot on every present enemy
    /// of the same type (including the dying one), so only one of each rare
    /// item drops per enemy type. The caller must not call this after an
    /// escape result (`FF9/btl_mot.cs:343`).
    pub fn set_bonus(
        &mut self,
        enemies: &mut [EnemyRewardData],
        index: usize,
        rng: &mut dyn BattleRandom,
    ) -> bool {
        if enemies[index].bonus_given {
            return false;
        }
        let type_id = enemies[index].type_id;
        self.gil = self.gil.wrapping_add(enemies[index].gil as i32);
        self.exp = self.exp.wrapping_add(enemies[index].exp);
        for slot in [3usize, 2, 1] {
            let e = &enemies[index];
            if e.items[slot] != NO_ITEM && rng.random8() < i32::from(e.item_rates[slot]) {
                self.items.push(e.items[slot]);
                for other in enemies.iter_mut() {
                    if other.present && other.type_id == type_id {
                        other.items[slot] = NO_ITEM;
                    }
                }
                break;
            }
        }
        let e = &enemies[index];
        if e.items[0] != NO_ITEM && rng.random8() < i32::from(e.item_rates[0]) {
            self.items.push(e.items[0]);
        }
        let e = &enemies[index];
        if self.card == NO_CARD && e.card != NO_CARD && rng.random8() < i32::from(e.card_rate) {
            self.card = e.card;
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Party side
// ---------------------------------------------------------------------------

/// `CharacterBaseStats` row (`Memoria.Patcher/StreamingAssets/Data/Characters/BaseStats.csv`;
/// the vanilla `_FF9Level_Base` row, `LevelBaseRow`, has the same numbers:
/// dex = Speed, str = Strength, mgc = Magic, wpr = Spirit, cap = Gems).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseStats {
    pub dex: u8,
    pub str_: u8,
    pub mgc: u8,
    pub wpr: u8,
    pub gems: u8,
}

/// `FF9LEVEL_BONUS` (`Global/ff9/FF9LEVEL_BONUS.cs`), a class mutated in place.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LevelBonus {
    pub dex: u16,
    pub str_: u16,
    pub mgc: u16,
    pub wpr: u16,
    pub cap: u16,
}

/// Level tables: `Leveling.csv` columns (`Memoria/Data/Characters/CharacterLevelUp.cs`).
/// `CharacterLevelUps[player.level].ExperienceToLevel` is the EXP threshold
/// of level `player.level + 1` (row 0 is 0, row 1 is 16:
/// `Memoria.Patcher/StreamingAssets/Data/Characters/Leveling.csv:7-8`). `bonus_hp[lv - 1]` and `bonus_mp[lv - 1]` feed
/// `FF9Level_GetHp` / `GetMp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelTables {
    exp_to_level: Vec<u32>,
    bonus_hp: Vec<u16>,
    bonus_mp: Vec<u16>,
}

impl LevelTables {
    /// Builds from the vanilla `_FF9Level_Exp` and `_FF9Level_HpMp` arrays
    /// (`tantalus_assets::pc::battle_tables::{level_exp, level_hp_mp}`).
    /// All three need at least [`LEVEL_COUNT`] rows
    /// (`ff9level.cs:57-58`).
    pub fn new(
        exp_to_level: Vec<u32>,
        bonus_hp: Vec<u16>,
        bonus_mp: Vec<u16>,
    ) -> Result<Self, RewardError> {
        if exp_to_level.len() < LEVEL_COUNT
            || bonus_hp.len() < LEVEL_COUNT
            || bonus_mp.len() < LEVEL_COUNT
        {
            return Err(RewardError::ShortLevelTable);
        }
        Ok(Self {
            exp_to_level,
            bonus_hp,
            bonus_mp,
        })
    }

    pub fn exp_for_next(&self, level: u8) -> u32 {
        self.exp_to_level[usize::from(level)]
    }
}

/// One learnable ability row (`CharacterAbility`: `Id`, `Ap`), `PA_DATA` in
/// the vanilla table (`AbilityPaRow`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaEntry {
    pub id: i32,
    pub max_ap: i32,
}

/// The `PLAYER` fields the results touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyMember {
    /// `member_flag` bit for this line is set (taking part in this battle).
    pub in_battle: bool,
    pub cur_hp: u32,
    pub cur_mp: u32,
    /// `status & DisableRewards` (Petrify, Virus, Zombie;
    /// `Memoria/Data/Battle/BattleStatus.cs:159`).
    pub rewards_disabled: bool,
    pub level: u8,
    pub exp: u32,
    pub base: BaseStats,
    pub bonus: LevelBonus,
    /// `basis.dex/str/mgc/wpr`.
    pub basis: [u8; 4],
    pub max_hp: u32,
    pub max_mp: u32,
    pub max_capa: u32,
    pub cur_capa: u32,
    /// `elem.dex/str/mgc/wpr` (basis plus equipment, capped by [`STAT_MAX`]).
    pub elem: [u8; 4],
    /// Sum over the 5 equip slots of the item stat bonus
    /// (`FF9Level_GetEquipBonus`, `ff9level.cs:197-224`): dex, str, mgc, wpr.
    pub equip_stat_bonus: [i32; 4],
    /// `itemData.ability` of every equipped item, slot order, zeros kept
    /// (zero is skipped like the source, `BattleResultUI.cs:623`).
    pub equipped_abilities: Vec<i32>,
    /// `_FF9Abil_PaData[PresetId]`.
    pub learnable: Vec<PaEntry>,
    /// `player.pa`, one entry per `learnable` row.
    pub pa: Vec<i32>,
}

impl PartyMember {
    /// `ff9abil.FF9Abil_HasAp` (`Memoria/Data/ff9abil.cs:169-174`).
    pub fn has_ap(&self) -> bool {
        self.learnable.iter().any(|p| p.max_ap > 0)
    }

    /// `BattleResultUI.IsEnable` (`BattleResultUI.cs:332-335`).
    pub fn is_enabled(&self) -> bool {
        self.cur_hp > 0 && !self.rewards_disabled
    }
}

/// The party state the results write to. No I/O.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartySnapshot {
    pub gil: u32,
    pub members: [Option<PartyMember>; 4],
    /// Inventory stacks `(item id, count)` (`FF9.item`).
    pub items: Vec<(u32, u32)>,
    /// Cards won, left for the Tetra Master collection (not modeled).
    pub cards: Vec<u32>,
}

// ---------------------------------------------------------------------------
// Reward
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewardError {
    ShortLevelTable,
    AlreadyApplied,
}

/// What one party line receives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemberShare {
    pub exp: Option<u32>,
    pub ap: Option<u16>,
}

/// Typed result of a battle, computed once and applied once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleReward {
    pub shares: [MemberShare; 4],
    pub gil: u32,
    /// Distinct stacks `(id, count)` in first-seen order, at most
    /// [`RESULT_ITEM_MAX`] distinct ids (`AnalyzeBonusItem`).
    pub items: Vec<(u32, u32)>,
    pub card: Option<u32>,
    applied: bool,
}

/// Effects of [`BattleReward::apply`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// Levels gained per line.
    pub levels_gained: [u32; 4],
    /// Ability ids that reached max AP, per line, in learn order.
    pub learned: [Vec<i32>; 4],
    /// Lines whose stone cost now exceeds the stone stock, so the source runs
    /// `FF9Play_UpdateSA` (support-ability disabling, not modeled).
    pub needs_sa_revalidation: [bool; 4],
    pub gil_added: u32,
    /// Item units that did not fit a 99 stack and were lost.
    pub items_overflow: u32,
}

impl BattleReward {
    /// A battle with nothing to pay (escape, enemy flee, event battle).
    pub fn none() -> Self {
        Self {
            shares: [MemberShare::default(); 4],
            gil: 0,
            items: Vec::new(),
            card: None,
            applied: false,
        }
    }

    /// `BattleResultUI.InitialInfo` / `AnalyzeParty` / `AnalyzeArgument` /
    /// `AnalyzeBonusItem` (`BattleResultUI.cs:395-523`) for a Victory.
    ///
    /// * A line counts when its `member_flag` bit is set and the member is
    ///   enabled (`hp > 0`, not Petrify/Virus/Zombie).
    /// * EXP: only lines with `exp < 9999999` count; each gets
    ///   `bonus.exp / count` (integer division, remainder lost); `0` when no
    ///   line counts (`:514`).
    /// * AP: every enabled line whose preset has any AP row gets the full
    ///   `bonus.ap` (`:515`, `:349`).
    /// * Gil: `(UInt32)bonus.gil` to the party (`:516`, `:437`).
    /// * Items: stacks per distinct id; when a new id would be the 7th, the
    ///   scan stops entirely (`:497-500`, a bare `return`).
    /// * An `AfterEvent` scene shows no result screen (`:35-37`
    ///   `InitialEvent`), so it pays nothing here.
    pub fn from_bonus(bonus: &BattleBonus, party: &PartySnapshot) -> Self {
        if bonus.event {
            return Self::none();
        }
        let mut need_exp = [false; 4];
        let mut need_ap = [false; 4];
        let mut remain = 0u32;
        for i in 0..4 {
            if bonus.member_flag & (1 << i) != 0
                && let Some(p) = &party.members[i]
            {
                need_exp[i] = p.is_enabled() && p.exp < EXP_CAP;
                need_ap[i] = p.is_enabled() && p.has_ap();
                if need_exp[i] {
                    remain += 1;
                }
            }
        }
        let default_exp = bonus.exp.checked_div(remain).unwrap_or(0);
        let mut shares = [MemberShare::default(); 4];
        for i in 0..4 {
            if need_exp[i] {
                shares[i].exp = Some(default_exp);
            }
            if need_ap[i] {
                shares[i].ap = Some(bonus.ap);
            }
        }
        let mut items: Vec<(u32, u32)> = Vec::new();
        for &id in &bonus.items {
            if id == NO_ITEM {
                continue;
            }
            if let Some(stack) = items.iter_mut().find(|(i, _)| *i == id) {
                stack.1 += 1;
            } else {
                if items.len() >= RESULT_ITEM_MAX {
                    break;
                }
                items.push((id, 1));
            }
        }
        Self {
            shares,
            gil: bonus.gil as u32,
            items,
            card: (bonus.card != NO_CARD).then_some(bonus.card),
            applied: false,
        }
    }

    pub fn is_applied(&self) -> bool {
        self.applied
    }

    /// Applies the reward to `party`: the second call returns
    /// [`RewardError::AlreadyApplied`] and changes nothing.
    ///
    /// Order follows the result screen: EXP and AP per line (EXP raises the
    /// level through `FF9Play_GrowLevel` each time the next threshold is
    /// reached), then items (`UpdateItem`), then gil (`UpdateGil`).
    pub fn apply(
        &mut self,
        party: &mut PartySnapshot,
        tables: &LevelTables,
    ) -> Result<ApplyReport, RewardError> {
        if self.applied {
            return Err(RewardError::AlreadyApplied);
        }
        self.applied = true;
        let mut report = ApplyReport::default();
        for i in 0..4 {
            let Some(member) = party.members[i].as_mut() else {
                continue;
            };
            if let Some(exp) = self.shares[i].exp {
                report.levels_gained[i] += add_exp(member, exp, tables, &mut report, i);
            }
            if let Some(ap) = self.shares[i].ap {
                add_ap(member, ap, &mut report.learned[i]);
            }
        }
        for &(id, count) in &self.items {
            report.items_overflow += add_item(&mut party.items, id, count);
        }
        if let Some(card) = self.card {
            party.cards.push(card);
        }
        let room = GIL_CAP.saturating_sub(party.gil);
        let added = self.gil.min(room);
        party.gil += added;
        report.gil_added = added;
        Ok(report)
    }
}

/// `UpdateExp` (`BattleResultUI.cs:525-559`) summed over its ticks:
/// `exp = min(exp + gain, 9999999)`, then `GrowLevel` while
/// `level < 99` and `exp >= ExperienceToLevel[level]`. Returns levels gained.
fn add_exp(
    m: &mut PartyMember,
    gain: u32,
    tables: &LevelTables,
    report: &mut ApplyReport,
    line: usize,
) -> u32 {
    if m.exp >= EXP_CAP || gain == 0 {
        return 0;
    }
    m.exp = (u64::from(m.exp) + u64::from(gain)).min(u64::from(EXP_CAP)) as u32;
    let mut gained = 0;
    while usize::from(m.level) < LEVEL_COUNT && tables.exp_for_next(m.level) <= m.exp {
        if grow_level(m, tables) {
            report.needs_sa_revalidation[line] = true;
        }
        gained += 1;
    }
    gained
}

/// `FF9Play_GrowLevel` -> `FF9Play_Build(player, level + 1, init: false,
/// lvup: true)` (`Global/ff9play.cs:439-443`, `:215-261`) with the stat getters
/// of `Global/ff9level.cs:67-195`. Returns true when the old stone usage
/// exceeds the new stock (the source then runs `FF9Play_UpdateSA`).
///
/// `FF9Play_Build` rebuilds `cur` from zero before calling the getters, so
/// their `cur.capa != 0 ? 0 : n` bonus is always `n` (the source comments say
/// so at `ff9level.cs:158-161`): dex +0, str +3, mgc +3, wpr +1, stones +5,
/// each plus the equipment stat bonus, stored in `u16` fields (wrapping
/// cast). Then `stat = min(cap, base + level * k + (bonus >> 5))` with
/// k = 1/10 dex, 3/10 str and mgc, 3/20 wpr, 4/10 stones; HP
/// `BonusHP[lv-1] * str / 50` capped 9999; MP `BonusMP[lv-1] * mgc / 100`
/// capped 999. Current HP/MP only shrink to the new maximum (`:257-259`).
fn grow_level(m: &mut PartyMember, tables: &LevelTables) -> bool {
    let lv = i32::from(m.level) + 1;
    let old_use = m.max_capa.wrapping_sub(m.cur_capa);
    let old_hp = m.cur_hp;
    let old_mp = m.cur_mp;
    m.level += 1;
    m.bonus.dex = m.bonus.dex.wrapping_add(m.equip_stat_bonus[0] as u16);
    m.bonus.str_ = m
        .bonus
        .str_
        .wrapping_add((3 + m.equip_stat_bonus[1]) as u16);
    m.bonus.mgc = m.bonus.mgc.wrapping_add((3 + m.equip_stat_bonus[2]) as u16);
    m.bonus.wpr = m.bonus.wpr.wrapping_add((1 + m.equip_stat_bonus[3]) as u16);
    m.bonus.cap = m.bonus.cap.wrapping_add(5);
    let b = m.base;
    let dex = (i32::from(b.dex) + lv / 10 + i32::from(m.bonus.dex >> 5)).min(50);
    let str_ = (i32::from(b.str_) + lv * 3 / 10 + i32::from(m.bonus.str_ >> 5)).min(99);
    let mgc = (i32::from(b.mgc) + lv * 3 / 10 + i32::from(m.bonus.mgc >> 5)).min(99);
    let wpr = (i32::from(b.wpr) + lv * 3 / 20 + i32::from(m.bonus.wpr >> 5)).min(50);
    m.basis = [dex as u8, str_ as u8, mgc as u8, wpr as u8];
    let idx = (lv - 1) as usize;
    m.max_hp = (u32::from(tables.bonus_hp[idx]) * u32::from(m.basis[1]) / 50).min(HP_MAX);
    m.max_mp = (u32::from(tables.bonus_mp[idx]) * u32::from(m.basis[2]) / 100).min(MP_MAX);
    m.max_capa = (i32::from(b.gems) + lv * 4 / 10 + i32::from(m.bonus.cap >> 5)) as u32;
    m.cur_capa = m.max_capa.wrapping_sub(old_use);
    m.cur_hp = old_hp.min(m.max_hp);
    m.cur_mp = old_mp.min(m.max_mp);
    // FF9Play_Update (`ff9play.cs:263-334`): elem = basis + equipment, capped.
    for (k, max) in STAT_MAX.iter().enumerate() {
        let v = i32::from(m.basis[k]) + m.equip_stat_bonus[k];
        m.elem[k] = v.clamp(0, i32::from(*max)) as u8;
    }
    old_use > m.max_capa
}

/// `AddAp` run once per AP point (`BattleResultUI.cs:561-587`; the AP
/// `BattleEndValue.step` is 1, `:446-451`, so each tick adds 1 and an ability
/// carried by two equipped items rises twice per tick). For every non-zero
/// ability of every equipped item: skip when not in the preset; when
/// `max > cur`: set to max and record the learn if `cur + ap >= max`, else add.
fn add_ap(m: &mut PartyMember, ap: u16, learned: &mut Vec<i32>) {
    if !m.has_ap() {
        return;
    }
    for _ in 0..ap {
        for k in 0..m.equipped_abilities.len() {
            let abil = m.equipped_abilities[k];
            if abil == 0 {
                continue;
            }
            let Some(idx) = m.learnable.iter().position(|p| p.id == abil) else {
                continue;
            };
            let max_ap = m.learnable[idx].max_ap;
            let cur = m.pa[idx];
            if max_ap > cur {
                if max_ap <= cur + 1 {
                    m.pa[idx] = max_ap;
                    learned.push(abil);
                } else {
                    m.pa[idx] += 1;
                }
            }
        }
    }
}

/// `ff9item.FF9Item_Add` (`Memoria/Data/ff9item.2.cs:244-269`): count capped
/// at 99, an existing stack tops up to 99, a new stack takes a free or new
/// slot. Returns the units lost to the cap.
fn add_item(items: &mut Vec<(u32, u32)>, id: u32, count: u32) -> u32 {
    if id == NO_ITEM {
        return 0;
    }
    let want = count.min(ITEM_COUNT_MAX);
    if let Some(stack) = items.iter_mut().find(|(i, c)| *i == id && *c > 0) {
        let room = ITEM_COUNT_MAX - stack.1;
        let add = want.min(room);
        stack.1 += add;
        return count - add;
    }
    items.push((id, want));
    count - want
}

// ---------------------------------------------------------------------------
// Flee bookkeeping
// ---------------------------------------------------------------------------

/// Result of an escape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscapeOutcome {
    /// Gil taken from the party (0 unless the Flee ability ended the battle).
    pub gil_lost: u32,
    /// Always [`BattleReward::none`]: `GoToBattleResult` runs
    /// `ClearBattleBonus` on an Escape result before the `escape_gil` check,
    /// so nothing is left to pay (`BattleHUD.Public.cs:743-756`).
    pub reward: BattleReward,
}

/// Escape bookkeeping (`battle.cs:446-473`, `BattleHUD.Public.cs:734-760`).
///
/// Only the Flee ability (`BTL_FLAG_ABILITY_FLEE`) costs gil: a tenth of
/// `bonus.gil` plus the `BonusGil` of every enemy still in the unit list,
/// capped at the party's gil (see [`flee_gil_loss`]). The party flee key
/// costs nothing. The bonus is consumed, so it cannot be paid later.
pub fn settle_escape(
    bonus: BattleBonus,
    flee_ability: bool,
    enemy_bonus_gil_sum: u32,
    party: &mut PartySnapshot,
) -> EscapeOutcome {
    let mut gil_lost = 0;
    if flee_ability {
        let (lost, left) = flee_gil_loss(bonus.gil as u32, enemy_bonus_gil_sum, party.gil);
        gil_lost = lost;
        party.gil = left;
    }
    EscapeOutcome {
        gil_lost,
        reward: BattleReward::none(),
    }
}

/// How a battle that ends because no enemy is left ends
/// (`battle.cs:193`): Victory when `bonus.exp > 0`, otherwise the enemies
/// fled and nothing is paid.
pub fn enemies_gone_reward(bonus: &BattleBonus, party: &PartySnapshot) -> BattleReward {
    if bonus.exp > 0 {
        BattleReward::from_bonus(bonus, party)
    } else {
        BattleReward::none()
    }
}
