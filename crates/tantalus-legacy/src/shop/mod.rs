//! Pure, deterministic shop, synthesis, inventory, equipment and out-of-battle
//! item rules (milestones G9 shops part and G4 inventory).
//!
//! No UI, no I/O, no clock, no global state. The caller supplies plain data
//! structs ([`ItemDb`], [`ShopTable`], [`Recipe`]) that mirror the item,
//! shop and synthesis tables; this crate does not depend on `tantalus-assets`.
//!
//! Source: Memoria pin `c716e1fa` (`E:/FFIX Recomp/.reference/Memoria`). Paths
//! in doc comments are relative to `Assembly-CSharp/`. The pinned code is
//! Memoria's, not the untouched vanilla binary. Memoria patch files
//! (`ShopPatch`, `ItemEquipPatch`, `ItemAbilityPatch`) and mod scripts are not
//! modeled. See `audit/playable-milestones.md` rows G4 and G9 for what is and
//! is not ported.

pub mod data;
pub mod equip;
pub mod field_item;
pub mod inventory;
pub mod trade;

pub use data::{
    ArmorData, ItemData, ItemDb, Recipe, ShopTable, StatBonus, default_selling_price,
    shops_from_flags,
};
pub use equip::{
    AbilityLearn, EquipError, EquipStats, Equipment, ShopComparison, abilities_of, can_equip,
    equip_candidates, equip_item, equip_mask_from_char_id, equip_part_of,
    equipped_support_abilities, learn_state, shop_comparison, shop_defence, stats, unequip,
};
pub use field_item::{FieldEffect, FieldItemError, FieldTarget, ItemUseOutcome, use_item};
pub use inventory::{GenericError, Inventory, Slot};
pub use trade::{
    ShopKind, TradeError, buy, can_synthesize, item_enabled, max_buy_count, max_sell_count,
    max_synth_count, sell, shop_kind, synthesis_list, synthesize,
};

/// `ff9item.FF9ITEM_COUNT_MAX` (`Memoria/Data/ff9item.2.cs:40`).
pub const ITEM_COUNT_MAX: i32 = 99;
/// `RegularItem.NoItem` (`Memoria/Data/Battle/RegularItem.cs:5`).
pub const NO_ITEM: u16 = 255;
/// Gil cap applied when selling (`Global/ShopUI.cs:450-451`, `9999999u`).
pub const GIL_MAX: u32 = 9_999_999;
/// Equipment slot count (`ff9feqp.cs:73-80`, `ff9play.cs:277`).
pub const EQUIP_SLOTS: usize = 5;
