//! Chocograph and Chocobo rules that live in Memoria's C# (milestone G9 remainder).
//!
//! Pure and device-free: no UI, I/O, clock or global state. State is the
//! 2048-byte `gEventGlobal` ([`EventGlobal`](crate::saves::ff9_tree::EventGlobal))
//! plus the one `hintmap_id` field.
//!
//! Source: Memoria pin `c716e1fa`; paths are relative to `Assembly-CSharp/`.
//!
//! What the C# contains is small. Chocobo Hot and Cold, feeding, chocobo
//! stats (beak level, speed, stamina) and the treasure contents (which item,
//! gil or card each dig spot gives) are field scripts, not C#. The C# only
//! reads and writes a few `gEventGlobal` bytes:
//!
//! | Byte or bit | Meaning | Read at |
//! | --- | --- | --- |
//! | 139 (Byte, Global) | beak level, read for the Lv99 achievement | `Global/EMinigame.cs:282` |
//! | 181 | Gysahl Greens already used flag | `Global/ItemUI.cs:47` |
//! | 184..=186 | Chocograph treasure opened, 24 bits | `Global/ChocographUI.cs:251` |
//! | 187..=189 | Chocograph found, 24 bits | `Global/ChocographUI.cs:250` |
//! | 190 | world map transport number (`w_moveCHRControl_No`) | `Global/ff9/ff9.cs:5665` |
//! | 191 | chocobo kind, also the Chocograph reach level | `Global/ChocographUI.cs:245`, `Global/ItemUI.cs:48` |
//!
//! Plus `FF9StateGlobal.hintmap_id` (saved in `70000_World`, script variable
//! 198 via `ff9.w_frameGetParameter`). Not in any C# file: which Chocograph
//! picture is at which dig spot, the reward, and the Hot and Cold distance
//! bands. The dig reward and Hot and Cold scoring are unmodeled here.

pub mod chocograph;
pub mod world;

pub use chocograph::{
    ChocographBook, ChocographEntry, HINT_MAP_MAX, Icon, REQUIREMENT, SelectError, TreasureTracker,
    all_treasure_cheat, treasure_hunter_rank,
};
pub use world::{
    ChocoboKind, ForestHud, MinigameHud, beak_achievement_level, chocobo_check,
    fat_chocobo_opponent, forest_dialog_disables_collider, forest_hud, is_riding_chocobo,
    is_walking_in_forest, is_walking_or_flying_in_forest, special_hud_for_message,
    use_gysahl_greens,
};
