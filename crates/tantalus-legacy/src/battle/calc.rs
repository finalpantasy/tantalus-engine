//! `BattleCalculator`, `CalcContext`, the caster/target helper methods and
//! `SBattleCalculator.CalcMain` / `CalcResult`.
//!
//! Source: `Memoria/Battle/Calculator/BattleCalculator.cs`, `CalcContext.cs`,
//! `BattleCaster.cs`, `BattleTarget.cs`, `SBattleCalculator.cs`,
//! `Global/btl_para.cs`.
//!
//! The C# keeps one `BTL_DATA` per unit and creates a separate
//! `BattleCaster` and `BattleTarget` wrapper per calculation (each with its
//! own `Flags`, `HpDamage`, `MpDamage`). Here units live in a slice and the
//! calculator holds the caster and target indices, which may be equal (a unit
//! targeting itself), so aliasing behaves like the C#.

use super::command::{BattleCommand, COMMAND_SUMMON_EIKO};
use super::rng::BattleRandom;
use super::scripts;
use super::status::{
    self, AlterResult, StatusCtx, StatusEnv, StatusTable, alter_status, alter_statuses,
    check_point_status, remove_status, remove_statuses,
};
use super::unit::{BattleUnit, Elements};

/// `BattleCalcFlags` (`BattleCalcFlags.cs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CalcFlags(pub u16);

impl CalcFlags {
    pub const MISS: u16 = 1;
    pub const DODGE: u16 = 2;
    pub const MP_ATTACK: u16 = 4;
    pub const ABSORB: u16 = 8;
    pub const GUARD: u16 = 64;
    pub const DIRECT_HP: u16 = 128;
    pub const ADD_STAT: u16 = 256;

    pub fn has(self, bit: u16) -> bool {
        self.0 & bit != 0
    }
}

/// `CalcFlag` (`CalcFlag.cs`), the per-unit flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnitFlags(pub u8);

impl UnitFlags {
    pub const HP_ALTERATION: u8 = 1;
    pub const HP_RECOVERY: u8 = 2;
    pub const CRITICAL: u8 = 4;
    pub const MP_ALTERATION: u8 = 8;
    pub const MP_RECOVERY: u8 = 16;
    pub const HP_DAMAGE_OR_HEAL: u8 = 1 | 2;
    pub const MP_DAMAGE_OR_HEAL: u8 = 8 | 16;

    pub fn has(self, bit: u8) -> bool {
        self.0 & bit != 0
    }
}

/// `BattleUnit.Flags`, `HpDamage`, `MpDamage` of one wrapper object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnitCalc {
    pub flags: UnitFlags,
    pub hp_damage: i32,
    pub mp_damage: i32,
}

/// Facts the formulas read that come from the wider battle (geometry, scene
/// info, UI state). Defaults are all false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CalcEnv {
    /// `btl_scene.Info.ReverseAttack` (Ipsen's Castle, `BattleCalculator.cs:667`).
    pub reverse_attack: bool,
    /// `IsCasterSameDirectionTarget()` (`BattleCalculator.cs:321`): caster and
    /// target face the same way (back attack). Geometry, supplied by caller.
    pub caster_same_direction_as_target: bool,
    /// `btl_escape_key != 0` (`BattleTarget.IsRunningAway`, `BattleTarget.cs:268`).
    pub escape_key_pressed: bool,
    /// `UIManager.Battle.FF9BMenu_IsEnable()` (`SBattleCalculator.cs:347`).
    pub battle_menu_enabled: bool,
    pub status: StatusEnv,
}

/// Errors that stop the whole calculation. The C# throws and `CalcMain`
/// catches (`SBattleCalculator.cs:115`): no `CalcResult` runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalcError {
    /// A `%` by zero, with the source expression.
    DivideByZero(&'static str),
    /// A script id this port does not implement.
    UnportedScript(i32),
}

impl std::fmt::Display for CalcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CalcError::DivideByZero(what) => write!(f, "division by zero in {what}"),
            CalcError::UnportedScript(id) => write!(f, "battle script {id} is not ported"),
        }
    }
}

impl std::error::Error for CalcError {}

/// `CalcAttackBonus` (`CalcAttackBonus.cs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackBonus {
    Simple = 1,
    WillPower = 2,
    Dexterity = 3,
    Magic = 4,
    Random = 5,
    Level = 7,
}

/// Result of one `calc_main`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcOutcome {
    pub flags: CalcFlags,
    pub target: UnitCalc,
    pub caster: UnitCalc,
    pub added_statuses: u64,
    pub trance_increase: i16,
    /// `FIG_INFO_TROUBLE` set on the target (`RaiseTrouble`).
    pub trouble: bool,
    /// The calculation ended with a Miss that was a Dodge.
    pub dodged: bool,
}

pub struct Calculator<'a> {
    pub units: &'a mut [BattleUnit],
    pub caster: usize,
    pub target: usize,
    pub command: BattleCommand,
    pub env: CalcEnv,
    pub table: &'a StatusTable,
    // `CalcContext`
    pub status_rate: i32,
    pub attack_power: i32,
    pub defense_power: i32,
    pub evade: i32,
    pub attack: i32,
    pub hit_rate: i32,
    pub flags: CalcFlags,
    pub added_statuses: u64,
    pub trance_increase: i16,
    pub is_drain: bool,
    damage_modifier_count: i32,
    // `BattleCaster` / `BattleTarget` wrapper state
    pub caster_calc: UnitCalc,
    pub target_calc: UnitCalc,
    initial_check_point: u64,
    pub trouble: bool,
    /// `PerformCalcResult` (`BattleCalculator.cs:185`).
    pub perform_calc_result: bool,
}

type Rng<'r> = &'r mut dyn BattleRandom;

impl<'a> Calculator<'a> {
    /// `new BattleCalculator(caster, target, command)`
    /// (`BattleCalculator.cs:195`). Order of effects: the target wrapper
    /// records the check-point status (which may add or remove Low HP), then
    /// `TranceIncrease = (Int16)(Comn.random16() % Target.Will)` draws one
    /// random number (a zero Will throws after the draw).
    pub fn new(
        units: &'a mut [BattleUnit],
        caster: usize,
        target: usize,
        command: BattleCommand,
        env: CalcEnv,
        table: &'a StatusTable,
        rng: Rng<'_>,
    ) -> Result<Self, CalcError> {
        let mut calc = Self {
            units,
            caster,
            target,
            command,
            env,
            table,
            status_rate: 0,
            attack_power: 0,
            defense_power: 0,
            evade: 0,
            attack: 0,
            hit_rate: 0,
            flags: CalcFlags::default(),
            added_statuses: 0,
            trance_increase: 0,
            is_drain: false,
            damage_modifier_count: 0,
            caster_calc: UnitCalc::default(),
            target_calc: UnitCalc::default(),
            initial_check_point: 0,
            trouble: false,
            perform_calc_result: true,
        };
        calc.initial_check_point = calc.check_point(rng);
        let draw = rng.random16();
        let will = i32::from(calc.t().will);
        if will == 0 {
            return Err(CalcError::DivideByZero("random16() % Target.Will"));
        }
        calc.trance_increase = (draw % will) as i16;
        Ok(calc)
    }

    // ---- unit access -----------------------------------------------------

    pub fn c(&self) -> &BattleUnit {
        &self.units[self.caster]
    }

    pub fn t(&self) -> &BattleUnit {
        &self.units[self.target]
    }

    pub fn c_mut(&mut self) -> &mut BattleUnit {
        &mut self.units[self.caster]
    }

    pub fn t_mut(&mut self) -> &mut BattleUnit {
        &mut self.units[self.target]
    }

    fn same_unit(&self) -> bool {
        self.caster == self.target
    }

    fn check_point(&mut self, rng: Rng<'_>) -> u64 {
        let ctx = StatusCtx {
            table: self.table,
            env: &self.env.status,
        };
        check_point_status(&mut self.units[self.target], &ctx, rng)
    }

    /// `BattleTarget.AddededCheckPointStatuses` (`BattleTarget.cs:19`).
    pub fn added_check_point_statuses(&mut self, rng: Rng<'_>) -> u64 {
        self.check_point(rng) & !self.initial_check_point
    }

    // ---- CalcContext -----------------------------------------------------

    /// `CalcContext.EnsureAttack` (`CalcContext.cs:36`): raises `Attack` to 1.
    pub fn ensure_attack(&mut self) -> i32 {
        if self.attack < 1 {
            self.attack = 1;
        }
        self.attack
    }

    /// `PowerDifference`.
    pub fn power_difference(&self) -> i32 {
        self.attack_power.wrapping_sub(self.defense_power)
    }

    /// `EnsurePowerDifference`.
    pub fn ensure_power_difference(&self) -> i32 {
        self.power_difference().max(1)
    }

    pub fn is_absorb(&self) -> bool {
        self.flags.has(CalcFlags::ABSORB)
    }

    /// `DamageModifierCount` setter, default method (`CalcContext.cs:58`):
    /// each +1 is `Attack = Attack * 3 >> 1`; each -1 is `Attack >>= 1`.
    pub fn modifier_up(&mut self) {
        self.damage_modifier_count += 1;
        self.attack = self.attack.wrapping_mul(3) >> 1;
    }

    pub fn modifier_down(&mut self) {
        self.damage_modifier_count -= 1;
        self.attack >>= 1;
    }

    pub fn damage_modifier_count(&self) -> i32 {
        self.damage_modifier_count
    }

    /// `DecreaseAttackDrastically` default (`CalcContext.cs:83`).
    pub fn decrease_attack_drastically(&mut self) {
        self.attack = 1;
    }

    // ---- BattleCaster (BattleCaster.cs) ----------------------------------

    /// `Caster.SetMagicAttack` (`BattleCaster.cs:17`).
    pub fn caster_set_magic_attack(&mut self, rng: Rng<'_>) {
        let magic = i32::from(self.c().magic);
        let level = i32::from(self.c().level);
        let draw = rng.random16();
        self.attack = i32::from((magic + draw % (1 + ((level + magic) >> 3))) as i16);
    }

    /// `Caster.SetPhysicalAttack` (`BattleCaster.cs:22`).
    pub fn caster_set_physical_attack(&mut self, rng: Rng<'_>) {
        let strength = i32::from(self.c().strength);
        let level = i32::from(self.c().level);
        let draw = rng.random16();
        self.attack = i32::from((strength + draw % (1 + ((level + strength) >> 2))) as i16);
    }

    /// `Caster.SetLowPhysicalAttack` (`BattleCaster.cs:27`).
    pub fn caster_set_low_physical_attack(&mut self, rng: Rng<'_>) {
        let strength = i32::from(self.c().strength);
        let level = i32::from(self.c().level);
        let draw = rng.random16();
        self.attack = i32::from((strength + draw % (1 + ((level + strength) >> 3))) as i16);
    }

    /// `Caster.PhysicalPenaltyAndBonusAttack` (`BattleCaster.cs:32`).
    pub fn caster_physical_penalty_and_bonus_attack(&mut self) {
        if self.c().is_under_any_status(status::MINI) {
            self.decrease_attack_drastically();
        } else if self.c().is_under_any_status(status::BERSERK)
            || (self.c().is_player && self.c().is_under_any_status(status::TRANCE))
        {
            self.modifier_up();
        }
    }

    /// `Caster.EnemyTranceBonusAttack` (`BattleCaster.cs:40`).
    pub fn caster_enemy_trance_bonus_attack(&mut self) {
        if !self.c().is_player && self.c().is_under_any_status(status::TRANCE) {
            self.modifier_up();
        }
    }

    /// `Caster.PenaltyMini` (`BattleCaster.cs:47`).
    pub fn caster_penalty_mini(&mut self) {
        if self.c().is_under_any_status(status::MINI) {
            self.modifier_down();
        }
    }

    /// `Caster.PenaltyPhysicalHitRate` (`BattleCaster.cs:53`).
    pub fn caster_penalty_physical_hit_rate(&mut self) {
        if self
            .c()
            .is_under_any_status(status::BLIND | status::CONFUSE)
        {
            self.hit_rate /= 2;
        }
    }

    /// `Caster.BonusPhysicalEvade` (`BattleCaster.cs:66`).
    pub fn caster_bonus_physical_evade(&mut self) {
        if self
            .c()
            .is_under_any_status(status::TRANCE | status::VANISH)
        {
            self.evade = 0;
        }
    }

    /// `Caster.BonusWeaponElement` (`BattleCaster.cs:72`).
    pub fn caster_bonus_weapon_element(&mut self) {
        if self
            .c()
            .weapon_elements()
            .count_common(self.c().bonus_elements)
            > 0
        {
            self.modifier_up();
        }
    }

    // ---- BattleTarget (BattleTarget.cs) ----------------------------------

    /// `Target.SetMagicDefense` (`BattleTarget.cs:47`).
    pub fn target_set_magic_defense(&mut self) {
        self.defense_power = self.t().magic_defence;
    }

    /// `Target.SetPhysicalDefense` (`BattleTarget.cs:52`).
    pub fn target_set_physical_defense(&mut self) {
        self.defense_power = self.t().physical_defence;
    }

    /// `Target.PhysicalPenaltyAndBonusAttack` (`BattleTarget.cs:57`).
    pub fn target_physical_penalty_and_bonus_attack(&mut self) {
        self.target_penalty_defence_attack();
        self.target_bonus_sleep_or_mini_attack();
    }

    /// `Target.PenaltyDefenceAttack` (`BattleTarget.cs:63`).
    pub fn target_penalty_defence_attack(&mut self) {
        if self
            .t()
            .is_under_any_status(status::DEFEND | status::PROTECT)
        {
            self.modifier_down();
        }
    }

    /// `Target.PenaltyShellAttack` (`BattleTarget.cs:69`).
    pub fn target_penalty_shell_attack(&mut self) {
        if self.t().is_under_any_status(status::SHELL) {
            self.modifier_down();
        }
    }

    /// `Target.PenaltyShellHitRate` (`BattleTarget.cs:75`).
    pub fn target_penalty_shell_hit_rate(&mut self) {
        if self.t().is_under_any_status(status::SHELL) {
            self.hit_rate >>= 1;
        }
    }

    /// `Target.PenaltyDefenceHitRate` (`BattleTarget.cs:81`).
    pub fn target_penalty_defence_hit_rate(&mut self) {
        if self.t().is_under_any_status(status::DEFEND) {
            self.hit_rate /= 2;
        }
    }

    /// `Target.PenaltyBanishHitRate` (`BattleTarget.cs:94`).
    pub fn target_penalty_banish_hit_rate(&mut self) {
        if self.t().is_under_any_status(status::VANISH) {
            self.hit_rate = 0;
        }
    }

    /// `Target.PenaltyPhysicalEvade` (`BattleTarget.cs:100`).
    pub fn target_penalty_physical_evade(&mut self) {
        if self.t().is_under_any_status(status::PENALTY_EVADE) {
            self.evade = 0;
        }
    }

    fn target_bonus_sleep_or_mini_attack(&mut self) {
        if self.t().is_under_any_status(status::SLEEP | status::MINI) {
            self.modifier_up();
        }
    }

    /// `Target.CanGuardElement` (`BattleTarget.cs:229`).
    fn target_can_guard_element(&mut self, element: Elements) -> bool {
        if self.t().guard_elements.count_common(element) > 0 {
            self.flags.0 |= CalcFlags::GUARD;
            return true;
        }
        false
    }

    /// `Target.CanAttackElement(ElementSet)` (`BattleTarget.cs:249`): Guard
    /// stops the attack; Half lowers the modifier count, Absorb zeroes the
    /// defense and sets Absorb, Weak raises the count; Cold or Aqua removes
    /// Heat and Fire removes Freeze (`AlterStatuses`, `:260`).
    pub fn target_can_attack_element(&mut self, element: Elements) -> bool {
        if self.target_can_guard_element(element) {
            return false;
        }
        if self.t().half_elements.count_common(element) > 0 {
            self.modifier_down();
        }
        if self.t().absorb_elements.count_common(element) > 0 {
            self.flags.0 |= CalcFlags::ABSORB;
            self.defense_power = 0;
        }
        if self.t().weak_elements.count_common(element) > 0 {
            self.modifier_up();
        }
        if element.contains(Elements::COLD) || element.contains(Elements::AQUA) {
            self.target_remove_status_bits(status::HEAT);
        }
        if element.contains(Elements::FIRE) {
            self.target_remove_status_bits(status::FREEZE);
        }
        true
    }

    /// `Target.IsRunningAway` (`BattleTarget.cs:268`).
    pub fn target_is_running_away(&self) -> bool {
        self.t().is_player && self.env.escape_key_pressed
    }

    fn target_remove_status_bits(&mut self, mask: u64) -> u32 {
        let env = self.env.status;
        remove_statuses(&mut self.units[self.target], mask, &env)
    }

    /// `BattleUnit.TryRemoveStatuses` (`BattleUnit.cs:542`).
    pub fn target_try_remove_statuses(&mut self, mask: u64) -> bool {
        self.target_remove_status_bits(mask) == status::REMOVE_DONE
    }

    /// `Target.TryAlterStatuses` (`BattleTarget.cs:273`).
    pub fn target_try_alter_statuses(&mut self, mask: u64, change_context: bool, rng: Rng<'_>) {
        let prev = self.t().status_permanent | self.t().status_current;
        let ctx = StatusCtx {
            table: self.table,
            env: &self.env.status,
        };
        let result = alter_statuses(&mut self.units[self.target], mask, true, &ctx, rng);
        let now = self.t().status_permanent | self.t().status_current;
        self.added_statuses |= now & !prev;
        if change_context {
            if result == AlterResult::Resist {
                self.flags.0 |= CalcFlags::GUARD;
            } else if result == AlterResult::Invalid {
                self.flags.0 |= CalcFlags::MISS;
            }
        }
    }

    /// `Target.CanBeAttacked` (`BattleTarget.cs:301`).
    pub fn target_can_be_attacked(&mut self) -> bool {
        if !self
            .t()
            .is_under_any_status(status::PETRIFY | status::DEATH)
        {
            return true;
        }
        self.flags.0 |= CalcFlags::MISS;
        false
    }

    /// `Target.CanBeRevived` (`BattleTarget.cs:310`).
    pub fn target_can_be_revived(&mut self) -> bool {
        if self.t().is_under_any_status(status::PETRIFY) {
            self.flags.0 |= CalcFlags::MISS;
            return false;
        }
        if self.t().is_under_any_status(status::DEATH) && self.t().is_zombie() {
            self.flags.0 |= CalcFlags::MISS;
            return false;
        }
        true
    }

    /// `Target.CanBeHealed` (`BattleTarget.cs:327`).
    pub fn target_can_be_healed(&mut self) -> bool {
        if !self.t().is_under_any_status(status::DEATH) {
            return true;
        }
        self.flags.0 |= CalcFlags::MISS;
        false
    }

    /// `Target.CheckUnsafetyOrMiss` (`BattleTarget.cs:336`).
    pub fn target_check_unsafety_or_miss(&mut self) -> bool {
        if !self.t().is_under_any_status(status::EASY_KILL) {
            return true;
        }
        self.flags.0 |= CalcFlags::MISS;
        false
    }

    /// `Target.CheckUnsafetyOrGuard` (`BattleTarget.cs:345`).
    pub fn target_check_unsafety_or_guard(&mut self) -> bool {
        if !self.t().is_under_any_status(status::EASY_KILL) {
            return true;
        }
        self.flags.0 |= CalcFlags::GUARD;
        false
    }

    /// `Target.CheckIsPlayer` (`BattleTarget.cs:354`).
    pub fn target_check_is_player(&mut self) -> bool {
        if self.t().is_player {
            return true;
        }
        self.flags.0 |= CalcFlags::MISS;
        false
    }

    /// `Target.TryKillFrozen` (`BattleTarget.cs:363`). Memoria adds the
    /// `EasyKill` exception (comment at `:367`). The queued `SysStone`
    /// command removal is not modeled.
    pub fn target_try_kill_frozen(&mut self) -> bool {
        if !self.t().is_under_any_status(status::FREEZE)
            || self.t().is_under_any_status(status::PETRIFY)
        {
            return false;
        }
        if self.t().is_under_any_status(status::EASY_KILL) {
            return false;
        }
        self.t_mut().kill();
        true
    }

    // ---- BattleCalculator ------------------------------------------------

    /// `NormalMagicParams` (`BattleCalculator.cs:204`).
    pub fn normal_magic_params(&mut self, rng: Rng<'_>) {
        self.set_command_power();
        self.caster_set_magic_attack(rng);
        self.target_set_magic_defense();
    }

    /// `OriginalMagicParams` (`BattleCalculator.cs:211`).
    pub fn original_magic_params(&mut self, rng: Rng<'_>) {
        self.set_weapon_power();
        self.caster_set_low_physical_attack(rng);
        self.target_set_magic_defense();
    }

    /// `NormalPhysicalParams` (`BattleCalculator.cs:218`).
    pub fn normal_physical_params(&mut self, rng: Rng<'_>) {
        self.set_command_power();
        self.caster_set_physical_attack(rng);
        self.target_set_physical_defense();
    }

    /// `WeaponPhysicalParams()` (`BattleCalculator.cs:225`).
    pub fn weapon_physical_params(&mut self, rng: Rng<'_>) {
        self.set_weapon_power();
        self.caster_set_low_physical_attack(rng);
        self.target_set_physical_defense();
    }

    /// `WeaponPhysicalParams(bonus)` (`BattleCalculator.cs:232`). Draw order:
    /// `baseDamage` first, then (Random bonus only) the second draw.
    pub fn weapon_physical_params_bonus(
        &mut self,
        bonus: AttackBonus,
        rng: Rng<'_>,
    ) -> Result<(), CalcError> {
        let level = i32::from(self.c().level);
        let strength = i32::from(self.c().strength);
        let base_damage = rng.random16() % (1 + ((level + strength) >> 3));
        self.attack_power = self.c().weapon_power();
        self.target_set_physical_defense();
        match bonus {
            AttackBonus::Simple => self.attack = strength + base_damage,
            AttackBonus::WillPower => {
                self.attack = ((strength + i32::from(self.c().will)) >> 1) + base_damage;
            }
            AttackBonus::Dexterity => {
                self.attack = ((strength + i32::from(self.c().dexterity)) >> 1) + base_damage;
            }
            AttackBonus::Magic => {
                self.attack = ((strength + i32::from(self.c().magic)) >> 1) + base_damage;
            }
            AttackBonus::Random => {
                let draw = rng.random16();
                if strength == 0 {
                    return Err(CalcError::DivideByZero("random16() % Caster.Strength"));
                }
                self.attack = draw % strength + base_damage;
            }
            AttackBonus::Level => {
                self.attack_power += level;
                self.attack = strength + base_damage;
            }
        }
        Ok(())
    }

    /// `MagicAccuracy` (`BattleCalculator.cs:261`).
    pub fn magic_accuracy(&mut self) {
        let sum =
            self.command.hit_rate + (i32::from(self.c().magic) >> 2) + i32::from(self.c().level)
                - i32::from(self.t().level);
        self.hit_rate = i32::from(sum as i16);
        self.hit_rate = self.hit_rate.clamp(1, 100);
        self.evade = self.t().magic_evade;
    }

    /// `PhysicalAccuracy` (`BattleCalculator.cs:273`).
    pub fn physical_accuracy(&mut self) {
        self.hit_rate = 100;
        self.evade = self.t().physical_evade;
    }

    /// `CanAttackMagic` (`BattleCalculator.cs:279`).
    pub fn can_attack_magic(&mut self) -> bool {
        self.can_attack_elemental_command() && self.can_use_command_for_flight()
    }

    /// `CanAttackElementalCommand` (`BattleCalculator.cs:284`).
    pub fn can_attack_elemental_command(&mut self) -> bool {
        let element = self.command.element;
        self.target_can_attack_element(element)
    }

    /// `CanAttackWeaponElementalCommand` (`BattleCalculator.cs:289`).
    pub fn can_attack_weapon_elemental_command(&mut self) -> bool {
        let element = self.c().weapon_elements();
        self.target_can_attack_element(element)
    }

    /// `CanUseCommandForFlight` (`BattleCalculator.cs:294`).
    pub fn can_use_command_for_flight(&mut self) -> bool {
        if self.t().is_levitate() && self.command.is_ground() {
            self.flags.0 |= CalcFlags::MISS;
            return false;
        }
        true
    }

    /// `IsCasterNotTarget` (`BattleCalculator.cs:312`).
    pub fn is_caster_not_target(&mut self) -> bool {
        if !self.same_unit() {
            return true;
        }
        self.flags.0 |= CalcFlags::MISS;
        false
    }

    /// `PrepareHpDraining` (`BattleCalculator.cs:336`).
    pub fn prepare_hp_draining(&mut self) {
        self.target_calc.flags.0 |= UnitFlags::HP_ALTERATION;
        self.caster_calc.flags.0 |= UnitFlags::HP_ALTERATION;
        if self.t().is_zombie() {
            self.target_calc.flags.0 |= UnitFlags::HP_RECOVERY;
        } else {
            self.caster_calc.flags.0 |= UnitFlags::HP_RECOVERY;
        }
        self.is_drain = true;
    }

    /// `TryPhysicalHit` (`BattleCalculator.cs:389`). Draws: one `random16`
    /// for the hit rate, then (only if it passed and the target is not the
    /// caster) one for the evade. The support-ability `HitRateSetup` features
    /// are not modeled.
    pub fn try_physical_hit(&mut self, rng: Rng<'_>) -> bool {
        self.caster_penalty_physical_hit_rate();
        self.caster_bonus_physical_evade();
        self.target_penalty_physical_evade();
        self.target_penalty_defence_hit_rate();
        self.target_penalty_banish_hit_rate();
        // `Target` under Float adds `Configuration.Battle.FloatEvadeBonus`
        // (`BattleCalculator.cs:396`); the shipped default is 0, so no change.
        if self.hit_rate <= rng.random16() % 100 {
            self.flags.0 |= CalcFlags::MISS;
            return false;
        }
        if self.same_unit() || self.evade <= rng.random16() % 100 {
            return true;
        }
        self.flags.0 |= CalcFlags::MISS;
        if !self.t().covering {
            self.flags.0 |= CalcFlags::DODGE;
        }
        false
    }

    /// `TryMagicHit` (`BattleCalculator.cs:419`).
    pub fn try_magic_hit(&mut self, rng: Rng<'_>) -> bool {
        if self.hit_rate <= rng.random16() % 100 {
            self.flags.0 |= CalcFlags::MISS;
            return false;
        }
        if self.evade > rng.random16() % 100 {
            self.flags.0 |= CalcFlags::MISS;
            return false;
        }
        true
    }

    /// `TryDirectHPDamage` (`BattleCalculator.cs:441`).
    pub fn try_direct_hp_damage(&mut self) {
        if self.command.power == 0 {
            if self.t().is_zombie() {
                if self.target_can_be_attacked() {
                    let max = self.t().max_hp;
                    self.t_mut().set_current_hp(max);
                }
            } else if self.t().is_under_any_status(status::EASY_KILL) {
                self.flags.0 |= CalcFlags::GUARD;
            } else {
                self.t_mut().kill();
            }
            return;
        }
        if self.t().is_under_any_status(status::DEATH) {
            self.flags.0 |= CalcFlags::MISS;
            return;
        }
        self.flags.0 |= CalcFlags::DIRECT_HP;
        let power = self.command.power as u32;
        self.t_mut().set_current_hp(power);
    }

    /// `TryAlterMagicStatuses` (`BattleCalculator.cs:472`): always draws one
    /// `random16`, even when the status set is empty.
    pub fn try_alter_magic_statuses(&mut self, rng: Rng<'_>) {
        if self.command.hit_rate > rng.random16() % 100 {
            let mask = self.command.ability_status;
            self.target_try_alter_statuses(mask, false, rng);
        }
    }

    /// `TryAlterCommandStatuses` (`BattleCalculator.cs:483`).
    pub fn try_alter_command_statuses(&mut self, rng: Rng<'_>) {
        let mut mask = self.command.ability_status;
        if !self.command.is_short_summon && self.command.command_id == COMMAND_SUMMON_EIKO {
            mask |= status::PROTECT;
        }
        self.target_try_alter_statuses(mask, true, rng);
    }

    /// `TryRemoveAbilityStatuses` (`BattleCalculator.cs:492`).
    pub fn try_remove_ability_statuses(&mut self) {
        let mask = self.command.ability_status;
        if !self.target_try_remove_statuses(mask) {
            self.flags.0 |= CalcFlags::MISS;
        }
    }

    /// `TryRemoveItemStatuses` (`BattleCalculator.cs:498`); `ItemStatus` is
    /// the command item's status.
    pub fn try_remove_item_statuses(&mut self) {
        let mask = self.command.item.map_or(0, |i| i.status);
        if !self.target_try_remove_statuses(mask) {
            self.flags.0 |= CalcFlags::MISS;
        }
    }

    /// `IsTargetLevelMultipleOfCommandRate` (`BattleCalculator.cs:504`).
    pub fn is_target_level_multiple_of_command_rate(&mut self) -> Result<bool, CalcError> {
        if self.command.hit_rate == 0 {
            return Err(CalcError::DivideByZero("Target.Level % Command.HitRate"));
        }
        if i32::from(self.t().level) % self.command.hit_rate == 0 {
            return Ok(true);
        }
        self.flags.0 |= CalcFlags::MISS;
        Ok(false)
    }

    /// `CalcDamageCommon` (`BattleCalculator.cs:513`).
    pub fn calc_damage_common(&mut self) {
        self.target_calc.flags.0 |= UnitFlags::HP_ALTERATION;
        if self.is_absorb() {
            self.target_calc.flags.0 |= UnitFlags::HP_RECOVERY;
        }
    }

    /// `CalcPhysicalHpDamage` (`BattleCalculator.cs:525`).
    pub fn calc_physical_hp_damage(&mut self) {
        self.calc_damage_common();
        let attack = self.ensure_attack();
        self.target_calc.hp_damage = attack.wrapping_mul(self.ensure_power_difference());
    }

    /// `CalcHpDamage` (`BattleCalculator.cs:534`).
    pub fn calc_hp_damage(&mut self) {
        self.calc_damage_common();
        let attack = self.ensure_attack();
        let mut damage = attack.wrapping_mul(self.ensure_power_difference());
        if self.command.is_short_summon {
            damage = damage.wrapping_mul(2) / 3;
        }
        self.target_calc.hp_damage = damage;
    }

    /// `CalcMpDamage` (`BattleCalculator.cs:545`).
    pub fn calc_mp_damage(&mut self) {
        self.target_calc.flags.0 |= UnitFlags::MP_ALTERATION;
        if self.is_absorb() {
            self.target_calc.flags.0 |= UnitFlags::MP_RECOVERY;
            self.defense_power = 0;
        }
        let attack = self.ensure_attack();
        self.target_calc.mp_damage = self.power_difference().max(0).wrapping_mul(attack) >> 2;
    }

    /// `CalcProportionDamage` (`BattleCalculator.cs:561`).
    pub fn calc_proportion_damage(&mut self) {
        self.calc_damage_common();
        if self.attack > 100 {
            self.attack = 100;
        }
        let mut damage = (self.t().max_hp as i32).wrapping_mul(self.attack) / 100;
        if self.command.is_short_summon {
            damage = damage.wrapping_mul(2) / 3;
        }
        self.target_calc.hp_damage = damage;
    }

    /// `CalcHpMagicRecovery` (`BattleCalculator.cs:584`).
    pub fn calc_hp_magic_recovery(&mut self) {
        if !self.t().is_zombie() {
            self.target_calc.flags.0 |= UnitFlags::HP_RECOVERY;
        }
        self.calc_damage_common();
        self.target_calc.hp_damage = self.attack_power.wrapping_mul(self.attack);
    }

    /// `BonusBackstabAndPenaltyLongDistance` (`BattleCalculator.cs:644`).
    pub fn bonus_backstab_and_penalty_long_distance(&mut self) {
        if self.env.caster_same_direction_as_target || self.target_is_running_away() {
            self.modifier_up();
        }
        let row_gap = (i32::from(self.c().row) - i32::from(self.t().row)).abs();
        if row_gap > 1 && !self.c().has_long_range_weapon() && self.command.is_short_range {
            self.modifier_down();
        }
    }

    /// `PenaltyReverseAttack` (`BattleCalculator.cs:664`).
    pub fn penalty_reverse_attack(&mut self) {
        if !self.env.reverse_attack {
            return;
        }
        self.attack_power = 60 - self.attack_power;
        if self.attack_power < 1 {
            self.attack_power = 1;
        }
    }

    /// `TryCriticalHit` (`BattleCalculator.cs:675`). Draws nothing when a
    /// quarter of the caster's Will is 0; otherwise two draws: the first is
    /// reduced modulo that quarter, the second modulo 100.
    pub fn try_critical_hit(&mut self, rng: Rng<'_>) {
        if self.target_calc.flags.has(UnitFlags::CRITICAL) {
            return;
        }
        let quarter_will = i32::from(self.c().will) >> 2;
        if quarter_will != 0 {
            let first = rng.random16() % quarter_will;
            let bonus = i32::from(self.c().critical_rate_bonus);
            let resist = i32::from(self.t().critical_rate_resistance);
            if first + bonus - resist > rng.random16() % 100 {
                self.attack = self.attack.wrapping_mul(2);
                self.target_calc.hp_damage = self.target_calc.hp_damage.wrapping_mul(2);
                self.target_calc.mp_damage = self.target_calc.mp_damage.wrapping_mul(2);
                self.target_calc.flags.0 |= UnitFlags::CRITICAL;
            }
        }
    }

    /// `ConsumeMpAttack` (`BattleCalculator.cs:689`).
    pub fn consume_mp_attack(&mut self) {
        if self.flags.has(CalcFlags::MP_ATTACK) {
            let cost = (self.c().max_mp >> 3) as i32;
            let now = (self.c().cur_mp as i32).wrapping_sub(cost).max(0);
            self.c_mut().cur_mp = now as u32;
        }
    }

    /// `SetCommandPower` (`BattleCalculator.cs:695`).
    pub fn set_command_power(&mut self) {
        self.attack_power = self.command.power;
    }

    /// `SetWeaponPower` (`BattleCalculator.cs:700`), not a monster transform:
    /// `(Int16)(GetWeaponPower * Command.Power / 10)`.
    pub fn set_weapon_power(&mut self) {
        let value = self.c().weapon_power().wrapping_mul(self.command.power) / 10;
        self.attack_power = i32::from(value as i16);
    }

    /// `SetWeaponPowerSum` (`BattleCalculator.cs:708`).
    pub fn set_weapon_power_sum(&mut self) {
        self.attack_power =
            i32::from((self.c().weapon_power().wrapping_add(self.command.power)) as i16);
    }

    /// `SetCommandAttack` (`BattleCalculator.cs:713`).
    pub fn set_command_attack(&mut self) {
        self.attack = self.command.power;
    }

    /// `PenaltyCommandDividedAttack` (`BattleCalculator.cs:718`).
    pub fn penalty_command_divided_attack(&mut self) {
        if self.command.is_divided() {
            self.modifier_down();
        }
    }

    /// `PenaltyCommandDividedHitRate` (`BattleCalculator.cs:724`).
    pub fn penalty_command_divided_hit_rate(&mut self) {
        if self.command.is_divided() {
            self.hit_rate /= 2;
        }
    }

    /// `BonusElement` (`BattleCalculator.cs:784`).
    pub fn bonus_element(&mut self) {
        if self
            .command
            .element_for_bonus
            .count_common(self.c().bonus_elements)
            > 0
        {
            self.modifier_up();
        }
    }

    /// `RaiseTrouble` (`BattleCalculator.cs:811`).
    pub fn raise_trouble(&mut self) {
        if self.command.target_id == self.t().id
            && self
                .t()
                .is_under_any_status(status::APPLY_TROUBLE & !self.added_statuses)
        {
            self.trouble = true;
        }
    }
}

// ---- btl_para: applying damage ---------------------------------------------

/// `btl_para.SetDamage` (`btl_para.cs:118`) without animation: returns the
/// damage shown.
fn set_damage(unit: &mut BattleUnit, damage: i32) -> i32 {
    if unit.is_under_any_status(status::DEATH) || unit.is_under_any_status(status::PETRIFY) {
        return 0;
    }
    if i64::from(unit.cur_hp) > i64::from(damage) {
        let hp = unit.cur_hp.wrapping_sub(damage as u32);
        unit.set_current_hp(hp);
    } else {
        unit.set_current_hp(0);
    }
    damage
}

/// `btl_para.SetRecover` (`btl_para.cs:171`).
fn set_recover(unit: &mut BattleUnit, recover: u32) -> i32 {
    if unit.is_under_any_status(status::DEATH) || unit.is_under_any_status(status::PETRIFY) {
        return 0;
    }
    if unit.cur_hp.wrapping_add(recover) < unit.max_hp {
        unit.set_current_hp(unit.cur_hp.wrapping_add(recover));
    } else {
        unit.set_current_hp(unit.max_hp);
    }
    recover as i32
}

/// `btl_para.SetMpDamage` (`btl_para.cs:201`).
fn set_mp_damage(unit: &mut BattleUnit, damage: u32) -> i32 {
    if unit.is_under_any_status(status::DEATH) || unit.is_under_any_status(status::PETRIFY) {
        return 0;
    }
    if unit.cur_mp > damage {
        unit.cur_mp -= damage;
    } else {
        unit.cur_mp = 0;
    }
    damage as i32
}

/// `btl_para.SetMpRecover` (`btl_para.cs:224`).
fn set_mp_recover(unit: &mut BattleUnit, recover: u32) -> i32 {
    if unit.is_under_any_status(status::DEATH) || unit.is_under_any_status(status::PETRIFY) {
        return 0;
    }
    if unit.cur_mp.wrapping_add(recover) < unit.max_mp {
        unit.cur_mp = unit.cur_mp.wrapping_add(recover);
    } else {
        unit.cur_mp = unit.max_mp;
    }
    recover as i32
}

/// `SBattleCalculator.CheckDamageReaction` (`SBattleCalculator.cs:345`):
/// trance gauge change when an enemy damages a player. Ignores
/// `isDebug` and `NoAutoTrance` (both false by default).
fn check_damage_reaction(v: &mut Calculator<'_>, rng: Rng<'_>) {
    if !v.env.battle_menu_enabled {
        return;
    }
    if v.t().is_player == v.c().is_player {
        return;
    }
    if !v.t().has_trance_gauge
        || v.t().cur_hp == 0
        || v.t().is_under_any_status(status::CANNOT_TRANCE)
    {
        return;
    }
    let sum = i32::from(v.t().trance) + i32::from(v.trance_increase);
    if sum < 0 {
        v.t_mut().trance = 0;
        if v.t().is_under_any_status(status::TRANCE) {
            let env = v.env.status;
            remove_status(&mut v.units[v.target], 14, &env);
        }
    } else if sum < i32::from(u8::MAX) {
        let inc = v.trance_increase as u8;
        let trance = v.t().trance.wrapping_add(inc);
        v.t_mut().trance = trance;
    } else {
        v.t_mut().trance = u8::MAX;
        let ctx = StatusCtx {
            table: v.table,
            env: &v.env.status,
        };
        alter_status(&mut v.units[v.target], 14, false, &ctx, rng);
    }
}

/// `SBattleCalculator.CalcResult` (`SBattleCalculator.cs:141`), with
/// `CustomBattleFlagsMeaning = 0` and no mod overloads. Presentation (figures,
/// motions, voices, enemy counter requests) is dropped; the HP, MP, status and
/// trance-gauge changes are kept.
pub fn calc_result(v: &mut Calculator<'_>, rng: Rng<'_>) -> CalcOutcome {
    let added = v.added_check_point_statuses(rng);
    v.added_statuses |= added;
    v.consume_mp_attack();
    let mut dodged = false;
    if v.flags.has(CalcFlags::GUARD) {
        // `FIG_INFO_GUARD`.
    } else if v.flags.has(CalcFlags::MISS) {
        if v.t().cur_hp == 0 {
            v.flags.0 &= !CalcFlags::DODGE;
        }
        dodged = v.flags.has(CalcFlags::DODGE);
    } else {
        if v.flags.has(CalcFlags::ADD_STAT) && v.t().cur_hp > 0 {
            let weapon_status = v.c().weapon.map_or(0, |w| w.status);
            v.target_try_alter_statuses(weapon_status, false, rng);
        }
        if v.command.ability_category & 8 != 0 {
            let mask = status::REMOVE_ON_PHYSICALLY_ATTACKED & !v.added_statuses;
            v.target_remove_status_bits(mask);
        }
        if v.command.ability_category & 16 != 0 {
            let mask = status::REMOVE_ON_MAGICALLY_ATTACKED & !v.added_statuses;
            v.target_remove_status_bits(mask);
        }
        if v.added_statuses & status::DEATH != 0 {
            v.target_calc.flags.0 &= !UnitFlags::HP_DAMAGE_OR_HEAL;
        }
        damage_final_changes(v);
        apply_target(v, rng);
        apply_caster(v);
    }
    let added = v.added_check_point_statuses(rng);
    v.added_statuses |= added;
    CalcOutcome {
        flags: v.flags,
        target: v.target_calc,
        caster: v.caster_calc,
        added_statuses: v.added_statuses,
        trance_increase: v.trance_increase,
        trouble: v.trouble,
        dodged,
    }
}

/// `SBattleCalculator.DamageFinalChanges` (`SBattleCalculator.cs:378`).
fn damage_final_changes(v: &mut Calculator<'_>) {
    if v.target_calc.flags.0 == 0 {
        return;
    }
    let multiplier = v.command.reflect_multiplier_on_target(v.t().id);
    if v.target_calc.flags.has(UnitFlags::HP_ALTERATION) {
        v.target_calc.hp_damage = v.target_calc.hp_damage.wrapping_mul(multiplier);
    }
    if v.target_calc.flags.has(UnitFlags::MP_ALTERATION) {
        v.target_calc.mp_damage = v.target_calc.mp_damage.wrapping_mul(multiplier);
    }
}

fn apply_target(v: &mut Calculator<'_>, rng: Rng<'_>) {
    if v.target_calc.flags.0 != 0 {
        if v.target_calc.flags.has(UnitFlags::HP_ALTERATION) {
            let limit = i64::from(v.c().max_damage_limit);
            if i64::from(v.target_calc.hp_damage) > limit {
                v.target_calc.hp_damage = v.c().max_damage_limit as i32;
            }
            if v.target_calc.flags.has(UnitFlags::HP_RECOVERY) {
                let amount = v.target_calc.hp_damage as u32;
                v.target_calc.hp_damage = set_recover(v.t_mut(), amount);
            } else {
                let amount = v.target_calc.hp_damage;
                v.target_calc.hp_damage = set_damage(v.t_mut(), amount);
                check_damage_reaction(v, rng);
            }
            if v.is_drain {
                v.caster_calc.flags.0 |= UnitFlags::HP_ALTERATION;
                if !v.target_calc.flags.has(UnitFlags::HP_RECOVERY) {
                    v.caster_calc.flags.0 |= UnitFlags::HP_RECOVERY;
                } else {
                    v.caster_calc.flags.0 &= !UnitFlags::HP_RECOVERY;
                }
                v.caster_calc.hp_damage = v.target_calc.hp_damage;
            }
        }
        if v.target_calc.flags.has(UnitFlags::MP_ALTERATION) {
            let limit = i64::from(v.c().max_mp_damage_limit);
            if i64::from(v.target_calc.mp_damage) > limit {
                v.target_calc.mp_damage = v.c().max_mp_damage_limit as i32;
            }
            let amount = v.target_calc.mp_damage as u32;
            if v.target_calc.flags.has(UnitFlags::MP_RECOVERY) {
                v.target_calc.mp_damage = set_mp_recover(v.t_mut(), amount);
            } else {
                v.target_calc.mp_damage = set_mp_damage(v.t_mut(), amount);
            }
            if v.is_drain {
                v.caster_calc.flags.0 |= UnitFlags::MP_ALTERATION;
                if !v.target_calc.flags.has(UnitFlags::MP_RECOVERY) {
                    v.caster_calc.flags.0 |= UnitFlags::MP_RECOVERY;
                } else {
                    v.caster_calc.flags.0 &= !UnitFlags::MP_RECOVERY;
                }
                v.caster_calc.mp_damage = v.target_calc.mp_damage;
            }
        }
    } else if v.flags.has(CalcFlags::DIRECT_HP) {
        // Motion and `CheckDamageReaction` only; the latter needs `rng` for
        // the trance status. It does not draw.
        check_damage_reaction(v, rng);
    }
}

fn apply_caster(v: &mut Calculator<'_>) {
    if v.caster_calc.flags.0 == 0 {
        return;
    }
    if v.caster_calc.flags.has(UnitFlags::HP_ALTERATION) {
        let limit = i64::from(v.c().max_damage_limit);
        if i64::from(v.caster_calc.hp_damage) > limit {
            v.caster_calc.hp_damage = v.c().max_damage_limit as i32;
        }
        let amount = v.caster_calc.hp_damage;
        if v.caster_calc.flags.has(UnitFlags::HP_RECOVERY) {
            v.caster_calc.hp_damage = set_recover(v.c_mut(), amount as u32);
        } else {
            v.caster_calc.hp_damage = set_damage(v.c_mut(), amount);
        }
    }
    if v.caster_calc.flags.has(UnitFlags::MP_ALTERATION) {
        let limit = i64::from(v.c().max_mp_damage_limit);
        if i64::from(v.caster_calc.mp_damage) > limit {
            v.caster_calc.mp_damage = v.c().max_mp_damage_limit as i32;
        }
        let amount = v.caster_calc.mp_damage as u32;
        if v.caster_calc.flags.has(UnitFlags::MP_RECOVERY) {
            v.caster_calc.mp_damage = set_mp_recover(v.c_mut(), amount);
        } else {
            v.caster_calc.mp_damage = set_mp_damage(v.c_mut(), amount);
        }
    }
}

/// `SBattleCalculator.CalcMain` (`SBattleCalculator.cs:35`) with
/// `CustomBattleFlagsMeaning = 0`, no mod overloads and no support-ability
/// features. A command with the physical category bit (8) first tries to
/// shatter a frozen target (`:94`). Script 64 (no effect, `:108`) runs only
/// `CalcResult`. An unported script id is an error; the C# would run it.
pub fn calc_main(
    units: &mut [BattleUnit],
    caster: usize,
    target: usize,
    command: BattleCommand,
    env: CalcEnv,
    table: &StatusTable,
    rng: &mut dyn BattleRandom,
) -> Result<CalcOutcome, CalcError> {
    let script_id = command.script_id;
    let category = command.ability_category;
    let mut v = Calculator::new(units, caster, target, command, env, table, rng)?;
    if category & 8 != 0 && v.target_try_kill_frozen() {
        return Ok(calc_result(&mut v, rng));
    }
    scripts::perform(script_id, &mut v, rng)?;
    if v.perform_calc_result {
        Ok(calc_result(&mut v, rng))
    } else {
        Ok(CalcOutcome {
            flags: v.flags,
            target: v.target_calc,
            caster: v.caster_calc,
            added_statuses: v.added_statuses,
            trance_increase: v.trance_increase,
            trouble: v.trouble,
            dodged: false,
        })
    }
}
