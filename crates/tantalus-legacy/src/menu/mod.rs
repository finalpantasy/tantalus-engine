//! Rules of the Equip and Ability menus (milestone M1), device-free.
//!
//! No UI, no I/O, no clock. The caller supplies plain data: the shared item
//! tables ([`crate::shop::ItemDb`]), the support-ability table
//! ([`SupportTable`]) and one [`MenuCharacter`] per party member. Nothing
//! here draws, plays a sound or reads a button.
//!
//! Source: Memoria pin `c716e1fa` (`E:/FFIX Recomp/.reference/Memoria`).
//! Paths in doc comments are relative to `Assembly-CSharp/`:
//! `Global/EquipUI.cs`, `Global/AbilityUI.cs`, `Global/ff9feqp.cs`,
//! `Global/ff9play.cs`, `Global/btl_eqp.cs`, `Memoria/Data/ff9abil.cs`,
//! `Memoria/Data/Characters/CharacterEquipment.cs`. Stat sums, equip masks,
//! the shop defense mapping and the item list order are reused from
//! [`crate::shop`]; AP gain after a battle lives in
//! [`crate::battle::rewards`].
//!
//! Not modeled (see `audit/playable-milestones.md`, row M1): support-ability
//! feature scripts (`SupportingAbilityFeature`: HP and MP bonuses, forced,
//! banished and hidden sets are inputs only), `use_condition` expressions,
//! `BattleAbilityHelper.Patch` and `BattleCommandHelper.Patch`, field
//! ability effects (`SFieldCalculator.FieldCalcMain`), the Memoria
//! `MagicStoneStockFormula` option, `ItemEquipPatch`/`ItemAbilityPatch`,
//! every sprite, sound, text and layout call.

pub mod ability;
pub mod equip;

use std::collections::BTreeSet;

use crate::battle::rewards::PaEntry;
use crate::shop::{Equipment, NO_ITEM, equip_mask_from_char_id};

/// `RegularItem.Moonstone` (`Memoria/Data/Battle/RegularItem.cs:235`).
pub const MOONSTONE: u16 = 229;
/// `CharacterId.Quina` (`Memoria/Data/Characters/CharacterId.cs:12`).
pub const QUINA: u32 = 5;
/// `BattleStatusConst.CannotUseAbilityInMenu` = Petrify | Silence
/// (`Memoria/Data/Battle/BattleStatus.cs:154`; bit 0 and bit 3, `:10`, `:13`).
pub const CANNOT_USE_ABILITY_IN_MENU: u32 = 0x1 | 0x8;

/// `Configuration.Battle.LockEquippedAbilities` (`Memoria/Configuration/
/// Structure/BattleSection.cs:76`, default 0). Values 1 and 3 lock support
/// abilities to the equipment; 2 and 3 hide learned active abilities except
/// for Quina (`AbilityUI.CheckAAType`, `AbilityUI.cs:1257-1262`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MenuConfig {
    pub lock_support: bool,
    pub lock_active: bool,
}

impl MenuConfig {
    /// Maps the integer option the way the source tests it
    /// (`== 1 || == 3` for support, `== 2 || == 3` for active).
    pub fn from_lock_option(value: i32) -> Self {
        Self {
            lock_support: value == 1 || value == 3,
            lock_active: value == 2 || value == 3,
        }
    }
}

/// One `CharacterAbilityGems` row (`Memoria/Data/Characters/
/// CharacterAbilityGems.cs:11-30`): the gem cost and the boosted versions
/// (empty in the stock `AbilityGems.csv`, which has no boosted column data).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SupportRow {
    pub gems: i32,
    /// Support-ability indexes of the boosted versions, in order.
    pub boosted: Vec<i32>,
}

/// `ff9abil._FF9Abil_SaData`, indexed by support-ability index
/// (`ff9abil.cs:50`). The source requires rows 0..=63
/// (`ff9abil.cs:416`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SupportTable {
    pub rows: Vec<SupportRow>,
}

impl SupportTable {
    /// Gem cost (`CharacterAbilityGems.GemsCount`). The source throws on a
    /// missing row; here a missing row costs 0.
    pub fn gems(&self, sa: i32) -> i32 {
        usize::try_from(sa)
            .ok()
            .and_then(|i| self.rows.get(i))
            .map_or(0, |r| r.gems)
    }

    /// `ff9abil.GetBoostedAbilityList` (`ff9abil.cs:213`).
    pub fn boosted(&self, sa: i32) -> &[i32] {
        usize::try_from(sa)
            .ok()
            .and_then(|i| self.rows.get(i))
            .map_or(&[], |r| r.boosted.as_slice())
    }
}

/// The `PLAYER` fields the two menus read and write.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuCharacter {
    /// `PLAYER.Index` (`CharacterId`, `ff9feqp.GetCharacterEquipMask`).
    pub char_id: u32,
    /// `basis.dex/str/mgc/wpr` (`ff9play.cs:263-272`).
    pub basis: [i32; 4],
    /// `player.equip` (`CharacterEquipment`).
    pub equip: Equipment,
    /// `_FF9Abil_PaData[PresetId]` (`ff9abil.cs:49`).
    pub learnable: Vec<PaEntry>,
    /// `player.pa`, one entry per `learnable` row.
    pub pa: Vec<i32>,
    /// `player.saExtended` in the order the abilities were enabled. The
    /// source keeps a `HashSet`, whose enumeration order is unspecified; for
    /// a set without removals it is insertion order, which this models.
    pub enabled: Vec<i32>,
    /// `player.saForced`, `saBanish`, `saHidden` (filled by SA features,
    /// which are not modeled; inputs here).
    pub forced: BTreeSet<i32>,
    pub banished: BTreeSet<i32>,
    pub hidden: BTreeSet<i32>,
    /// `max.capa`, `cur.capa` and `basis.capa` (stone stock, stones left,
    /// stones left as `FF9Play_Update` restores them before the recount).
    pub max_capa: u32,
    pub cur_capa: u32,
    pub basis_capa: u32,
    pub cur_hp: u32,
    pub cur_mp: u32,
    /// `player.status`, bit n is `BattleStatusId` n.
    pub status: u32,
}

impl MenuCharacter {
    /// A character with an empty record and nothing equipped.
    pub fn new(char_id: u32) -> Self {
        Self {
            char_id,
            equip: [NO_ITEM; crate::shop::EQUIP_SLOTS],
            ..Self::default()
        }
    }

    /// `ff9feqp.GetCharacterEquipMask` (`ff9feqp.cs:14-17`).
    pub fn equip_mask(&self) -> u64 {
        equip_mask_from_char_id(self.char_id)
    }

    /// `ff9abil.FF9Abil_GetIndex` (`ff9abil.cs:183`): first row with the id.
    pub fn ability_index(&self, ability_id: i32) -> Option<usize> {
        self.learnable.iter().position(|p| p.id == ability_id)
    }

    /// `ff9abil.FF9Abil_IsEnableSA` (`ff9abil.cs:60-68`).
    pub fn is_enabled(&self, sa: i32) -> bool {
        self.enabled.contains(&sa)
    }
}
