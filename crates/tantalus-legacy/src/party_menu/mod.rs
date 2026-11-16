//! Rules of the Item, Party Setting, Order, Status, Name and Config menus
//! (milestone M2). Pure and device-free: no UI, no I/O, no clock, no global
//! state. The caller passes plain data and gets plain data back.
//!
//! Source: Memoria pin `c716e1fa` (`E:/FFIX Recomp/.reference/Memoria`). Paths
//! in doc comments are relative to `Assembly-CSharp/`. The pinned code is
//! Memoria's, not the untouched vanilla binary. Rendering, atlases, sounds,
//! tweens and cursor handling are not modeled; the types here are only the
//! decisions those screens take. `audit/playable-milestones.md` row M2 lists
//! what is and is not ported.
//!
//! Reused, not duplicated: [`crate::shop::Inventory`] (stack add, remove,
//! auto arrange, key items), [`crate::shop::use_item`] (field item math),
//! [`crate::shop::stats`] (equipment stat sums) and
//! [`crate::battle::rewards::LevelTables`] (EXP thresholds).

pub mod config;
pub mod item;
pub mod main_menu;
pub mod name;
pub mod party;
pub mod status;

/// `CharacterId.NONE` (`Memoria/Data/Characters/CharacterId.cs:19`).
pub const CHARACTER_NONE: u8 = 255;
/// `BattleStatusConst.BattleEndInMenu` = `BattleEnd & OutOfBattle`
/// (`Memoria/Data/Battle/BattleStatus.cs:140,142`). `BattleEnd` is Petrify
/// (bit 0), Venom (bit 1) and Stop; Stop is not in `OutOfBattle`, so only
/// Petrify and Venom remain.
pub const BATTLE_END_IN_MENU: u64 = 0b11;
