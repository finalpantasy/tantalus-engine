//! Pure, deterministic battle-calculation core (milestone B4).
//!
//! No I/O, no clock, no global state. Every random draw goes through the
//! [`BattleRandom`] trait, so a fixed draw sequence gives fixed outputs.
//!
//! Source: Memoria pin `c716e1fa` (`E:/FFIX Recomp/.reference/Memoria`), paths
//! below are relative to `Assembly-CSharp/` unless they start with
//! `Memoria.Scripts/`. The pinned code is Memoria's, not the untouched vanilla
//! binary: where a comment in the source says Memoria added or changed a
//! behavior it is noted in the doc comment of the function. Memoria
//! configuration switches are fixed at their shipped defaults
//! (`CustomBattleFlagsMeaning = 0`, `BreakDamageLimit = 0`,
//! `FloatEvadeBonus = 0`, empty `StatusDurationFormula` and
//! `StatusTickFormula`, no `IOverload*` mod scripts). Support-ability
//! "features" (`ff9abil.GetEnabledSA(...).TriggerOnAbility`) and cheat
//! settings (`IsDmg9999`, `IsHpMpFull`) are data driven or UI settings and are
//! not modeled; see `audit/playable-milestones.md` row B4.

pub mod atb;
pub mod calc;
pub mod command;
pub mod cost;
pub mod flee;
pub mod host;
pub mod machine;
pub mod queue;
pub mod rewards;
pub mod rng;
pub mod script_host;
pub mod scripts;
pub mod status;
pub mod targeting;
pub mod unit;

pub use calc::{
    AttackBonus, CalcEnv, CalcError, CalcFlags, CalcOutcome, Calculator, UnitCalc, UnitFlags,
    calc_main, calc_result,
};
pub use command::{BattleCommand, ItemEffect};
pub use rng::{BattleRandom, SequenceRandom, SplitMix64Random};
pub use status::{AlterResult, StatusCtx, StatusDataRow, StatusEnv, StatusTable};
pub use unit::{BattleUnit, Elements, WeaponStats};
