//! Effect scripts by `Script ID` (`AA_DATA` `btl_ref.prog_no`).
//!
//! Source: `Memoria.Scripts/Sources/Battle/NNNN_*.cs` (`Perform()` bodies) and
//! `BaseWeaponScript.cs`. Only `Perform` is ported; the `RateTarget`
//! estimators (enemy AI scoring) are separate code paths.
//!
//! Ported ids: 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 19 20 22 25 27 42 44
//! 48 55 58 62 63 64 69 70 71 72 73 74 75 76 77 96 100 103 109. Every other id returns
//! [`CalcError::UnportedScript`].

use super::calc::{AttackBonus, CalcError, CalcFlags, Calculator, EatResult, UnitFlags};
use super::rng::BattleRandom;
use super::status;

type Rng<'r> = &'r mut dyn BattleRandom;

/// Run script `id`. `Ok(())` means the script finished (a Miss or Guard is a
/// flag in the calculator, not an error).
pub fn perform(id: i32, v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    match id {
        1 => base_weapon(v, AttackBonus::Simple, rng),
        2 => base_weapon(v, AttackBonus::WillPower, rng),
        3 => base_weapon(v, AttackBonus::Dexterity, rng),
        4 => base_weapon(v, AttackBonus::Magic, rng),
        5 => base_weapon(v, AttackBonus::Random, rng),
        6 => blood_sword(v, rng),
        7 => base_weapon(v, AttackBonus::Level, rng),
        8 => enemy_physical_attack(v, rng),
        9 => magic_attack(v, rng),
        10 => magic_recovery(v, rng),
        11 => magic_apply_negative_status(v, rng),
        12 => magic_cure_status(v),
        13 => revive(v, rng),
        14 => death(v, rng),
        15 => drain_mp(v, rng),
        16 => drain_hp(v, rng),
        17 => magic_gravity_damage(v, rng),
        19 => physical_attack(v, rng),
        20 => original_magic_attack(v, rng),
        22 => lv_direct_hp_damage(v),
        25 => precise_direct_hp_damage(v),
        27 => direct_hp_damage(v, rng),
        42 => throw(v, rng),
        44 => focus(v),
        48 => spear(v, rng),
        55 => change_row(v),
        58 => steal(v, rng),
        62 => item_soft(v),
        63 => magic_sword_attack(v, rng),
        // `SBattleCalculator.cs:108`: no script, only `CalcResult`.
        64 => Ok(()),
        65 => eat(v),
        69 => item_potion(v, rng),
        70 => item_ether(v),
        71 => item_elixir(v),
        72 => item_phoenix(v, rng),
        73 => item_cure_status(v),
        74 => item_gem(v),
        75 => dead_pepper(v),
        76 => tent(v, rng),
        77 => dark_matter(v),
        96 => trance_full(v, rng),
        100 => precise_enemy_physical_attack(v, rng),
        103 => magic_apply_positive_status(v, rng),
        109 => mini(v, rng),
        other => Err(CalcError::UnportedScript(other)),
    }
}

/// Script 48 (`0048_SpearScript.cs`), the landing of a Jump.
fn spear(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    v.weapon_physical_params_bonus(AttackBonus::Simple, rng)?;
    v.modifier_up();
    v.caster_penalty_mini();
    v.target_physical_penalty_and_bonus_attack();
    v.calc_physical_hp_damage();
    Ok(())
}

/// Script 44 (`0044_FocusScript.cs`): Magic becomes
/// `min(99, Magic + Magic / Power)`. The C# applies it through the
/// `ChangeStat` status; here the stat is raised directly, which lasts the
/// battle as that status does.
fn focus(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    let power = v.command.power;
    if power == 0 {
        return Err(CalcError::DivideByZero("Target.Magic / Command.Power"));
    }
    let magic = i32::from(v.t().magic);
    v.t_mut().magic = (magic + magic / power).min(99) as u8;
    Ok(())
}

/// Script 65 (`0065_EatScript.cs`), Eat and Cook: a Humanoid, Easy Kill
/// guarded or unattackable target cannot be eaten; one above
/// `MaximumHp / Power` HP is too strong; otherwise it dies and its blue
/// magic is the eater's to learn.
fn eat(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    const HUMANOID: u8 = 1;
    if !v.target_check_unsafety_or_miss()
        || !v.target_can_be_attacked()
        || (!v.t().is_player && v.t().enemy_category & HUMANOID != 0)
    {
        v.eaten = Some(EatResult::CannotEat);
        return Ok(());
    }
    let power = v.command.power;
    if power == 0 {
        return Err(CalcError::DivideByZero("Target.MaximumHp / Command.Power"));
    }
    if i64::from(v.t().cur_hp) > i64::from(v.t().max_hp) / i64::from(power) {
        v.eaten = Some(EatResult::Failed);
        return Ok(());
    }
    let blue = v.t().blue_magic;
    v.t_mut().kill();
    v.eaten = Some(if blue == 0 {
        EatResult::TasteBad
    } else {
        EatResult::Yummy(blue)
    });
    Ok(())
}

/// Script 55 (`0055_ChangeRowScript.cs`): `btl_para.SwitchPlayerRow`
/// flips `bi.row` between 0 (back) and 1 (front). The model's 400-unit
/// step is the client's to draw.
fn change_row(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    let target = v.t_mut();
    target.row = if target.row == 0 { 1 } else { 0 };
    Ok(())
}

/// Script 58 (`0058_StealScript.cs`) without the Bandit and Master Thief
/// support abilities and the `StealingAlwaysWorks` cheat: a
/// `Level + Will` against `Target.Level` roll, then the rarest slot first,
/// each at its rate out of 256 (`BattleCalculator.StealItem` empties the
/// slot). A zero `Target.Level` would throw in the C# modulo; here the roll
/// passes.
fn steal(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    use super::unit::NO_ITEM;
    let items = v.t().steal_items;
    if items.iter().all(|&item| item == NO_ITEM) {
        v.stolen = Some(None);
        return Ok(());
    }
    v.hit_rate = i32::from(v.c().level) + i32::from(v.c().will);
    v.evade = i32::from(v.t().level);
    if v.hit_rate > 0 && v.evade > 0 && rng.random16() % v.hit_rate < rng.random16() % v.evade {
        v.stolen = Some(None);
        return Ok(());
    }
    let rates = v.t().steal_rates;
    for slot in (0..4).rev() {
        if rng.random8() < i32::from(rates[slot]) {
            let item = items[slot];
            if item == NO_ITEM {
                break;
            }
            v.t_mut().steal_items[slot] = NO_ITEM;
            v.stolen = Some(Some(item));
            return Ok(());
        }
    }
    v.stolen = Some(None);
    Ok(())
}

/// `BaseWeaponScript.Perform` (`BaseWeaponScript.cs:19`). Ids 1 (Simple),
/// 2 (Will), 3 (Dexterity), 4 (Magic), 5 (Random), 7 (Level) differ only in
/// `WeaponPhysicalParams(bonus)`.
fn base_weapon(v: &mut Calculator<'_>, bonus: AttackBonus, rng: Rng<'_>) -> Result<(), CalcError> {
    if v.target_try_kill_frozen() {
        return Ok(());
    }
    v.physical_accuracy();
    if !v.try_physical_hit(rng) {
        return Ok(());
    }
    v.weapon_physical_params_bonus(bonus, rng)?;
    v.caster_physical_penalty_and_bonus_attack();
    v.target_physical_penalty_and_bonus_attack();
    // Steiner (`CharacterId` 3) in Trance doubles the attack (`:31`).
    if v.c().is_under_any_status(status::TRANCE) && v.c().player_index() == 3 {
        v.attack = v.attack.wrapping_mul(2);
    }
    v.bonus_backstab_and_penalty_long_distance();
    v.caster_bonus_weapon_element();
    if v.can_attack_weapon_elemental_command() {
        v.try_critical_hit(rng);
        v.penalty_reverse_attack();
        v.calc_physical_hp_damage();
        v.raise_trouble();
    }
    Ok(())
}

/// Script 6, Blood Sword (`0006_BloodSwordWeaponScript.cs:20`).
fn blood_sword(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if !v.is_caster_not_target() || !v.target_can_be_attacked() || v.target_try_kill_frozen() {
        return Ok(());
    }
    v.physical_accuracy();
    if !v.try_physical_hit(rng) {
        return Ok(());
    }
    v.weapon_physical_params_bonus(AttackBonus::Simple, rng)?;
    v.caster_physical_penalty_and_bonus_attack();
    v.target_physical_penalty_and_bonus_attack();
    v.bonus_backstab_and_penalty_long_distance();
    v.try_critical_hit(rng);
    v.penalty_reverse_attack();
    v.prepare_hp_draining();
    v.calc_physical_hp_damage();
    v.caster_calc.hp_damage = v.target_calc.hp_damage;
    Ok(())
}

/// Script 8 (`0008_EnemyPhysicalAttackScript.cs:20`).
fn enemy_physical_attack(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if v.target_try_kill_frozen() {
        return Ok(());
    }
    v.physical_accuracy();
    if !v.try_physical_hit(rng) {
        return Ok(());
    }
    v.normal_physical_params(rng);
    v.caster_enemy_trance_bonus_attack();
    v.caster_physical_penalty_and_bonus_attack();
    v.target_physical_penalty_and_bonus_attack();
    v.bonus_backstab_and_penalty_long_distance();
    v.bonus_element();
    if !v.can_attack_elemental_command() {
        return Ok(());
    }
    v.try_critical_hit(rng);
    v.calc_physical_hp_damage();
    v.raise_trouble();
    v.try_alter_magic_statuses(rng);
    Ok(())
}

/// Script 100 (`0100_PreciseEnemyPhysicalAttackScript.cs`): no hit roll.
fn precise_enemy_physical_attack(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if v.target_try_kill_frozen() {
        return Ok(());
    }
    v.normal_physical_params(rng);
    v.caster_enemy_trance_bonus_attack();
    v.caster_physical_penalty_and_bonus_attack();
    v.target_physical_penalty_and_bonus_attack();
    v.bonus_backstab_and_penalty_long_distance();
    v.bonus_element();
    if !v.can_attack_elemental_command() {
        return Ok(());
    }
    v.calc_hp_damage();
    v.try_alter_magic_statuses(rng);
    Ok(())
}

/// Script 9, basic magic attack (`0009_MagicAttackScript.cs:18`).
fn magic_attack(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    v.normal_magic_params(rng);
    v.caster_enemy_trance_bonus_attack();
    v.caster_penalty_mini();
    v.target_penalty_shell_attack();
    v.penalty_command_divided_attack();
    v.bonus_element();
    if v.can_attack_magic() {
        v.calc_hp_damage();
        v.try_alter_magic_statuses(rng);
    }
    Ok(())
}

/// Script 10, Cure family (`0010_MagicRecoveryScript.cs:18`).
fn magic_recovery(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    v.normal_magic_params(rng);
    v.caster_enemy_trance_bonus_attack();
    v.caster_penalty_mini();
    v.penalty_command_divided_attack();
    v.calc_hp_magic_recovery();
    Ok(())
}

/// Script 11, negative status spells (`0011_MagicApplyNegativeStatusScript.cs:20`).
fn magic_apply_negative_status(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    v.magic_accuracy();
    v.target_penalty_shell_hit_rate();
    v.penalty_command_divided_hit_rate();
    if v.try_magic_hit(rng) {
        v.try_alter_command_statuses(rng);
    }
    Ok(())
}

/// Script 12, cure-status spells (`0012_MagicCureStatusScript.cs:21`).
fn magic_cure_status(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    v.try_remove_ability_statuses();
    Ok(())
}

/// Script 13, Life family (`0013_ReviveScript.cs`).
fn revive(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if !v.target_can_be_revived() {
        return Ok(());
    }
    // `HitRateForZombie() && !TryMagicHit()`.
    if v.t().is_zombie() {
        v.magic_accuracy();
        if !v.try_magic_hit(rng) {
            return Ok(());
        }
    }
    if v.t().is_zombie() {
        v.t_mut().kill();
        return Ok(());
    }
    if !v.target_check_is_player() {
        return Ok(());
    }
    v.target_calc.flags.0 |= UnitFlags::HP_ALTERATION | UnitFlags::HP_RECOVERY;
    // `(Int32)(MaximumHp * (Will + Power) / 100)`: uint * int is evaluated in
    // 64 bits in C#.
    let value = i64::from(v.t().max_hp) * (i64::from(v.t().will) + i64::from(v.command.power));
    v.target_calc.hp_damage = (value / 100) as i32;
    v.try_remove_ability_statuses();
    Ok(())
}

/// Script 14, Death spell (`0014_DeathScript.cs`).
fn death(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if !v.target_check_unsafety_or_guard() {
        return Ok(());
    }
    if v.t().is_zombie() {
        if v.target_can_be_attacked() {
            let max = v.t().max_hp;
            v.t_mut().set_current_hp(max);
        }
        return Ok(());
    }
    v.magic_accuracy();
    if v.try_magic_hit(rng) {
        v.try_alter_command_statuses(rng);
    }
    Ok(())
}

/// Script 15, Drain MP (`0015_DrainMpScript.cs`).
fn drain_mp(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if !v.is_caster_not_target() || !v.target_can_be_attacked() {
        return Ok(());
    }
    v.normal_magic_params(rng);
    v.caster_enemy_trance_bonus_attack();
    v.caster_penalty_mini();
    v.target_penalty_shell_attack();
    v.target_calc.flags.0 |= UnitFlags::MP_ALTERATION;
    v.caster_calc.flags.0 |= UnitFlags::MP_ALTERATION;
    v.is_drain = true;
    v.calc_mp_damage();
    let mut damage = v.target_calc.mp_damage;
    if v.t().is_zombie() {
        v.target_calc.flags.0 |= UnitFlags::MP_RECOVERY;
        if i64::from(damage) > i64::from(v.c().cur_mp) {
            damage = v.c().cur_mp as i32;
        }
    } else {
        v.caster_calc.flags.0 |= UnitFlags::MP_RECOVERY;
        if i64::from(damage) > i64::from(v.t().cur_mp) {
            damage = v.t().cur_mp as i32;
        }
    }
    v.target_calc.mp_damage = damage;
    v.caster_calc.mp_damage = damage;
    Ok(())
}

/// Script 16, Drain HP (`0016_DrainHpScript.cs`).
fn drain_hp(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if !v.is_caster_not_target() || !v.target_can_be_attacked() {
        return Ok(());
    }
    v.normal_magic_params(rng);
    v.caster_enemy_trance_bonus_attack();
    v.caster_penalty_mini();
    v.target_penalty_shell_attack();
    v.prepare_hp_draining();
    if v.power_difference() < 1 {
        return Ok(());
    }
    v.calc_hp_damage();
    v.caster_calc.hp_damage = v.target_calc.hp_damage;
    Ok(())
}

/// Script 17, Gravity-type proportional damage (`0017_MagicGravityDamageScript.cs`).
fn magic_gravity_damage(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if !v.target_check_unsafety_or_miss() {
        return Ok(());
    }
    v.magic_accuracy();
    v.target_penalty_shell_hit_rate();
    v.penalty_command_divided_hit_rate();
    if !v.try_magic_hit(rng) {
        return Ok(());
    }
    v.set_command_attack();
    v.bonus_element();
    if v.can_attack_magic() {
        v.calc_proportion_damage();
    }
    Ok(())
}

/// Script 19 (`0019_PhysicalAttackScript.cs:20`).
fn physical_attack(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    v.weapon_physical_params(rng);
    v.caster_enemy_trance_bonus_attack();
    v.caster_physical_penalty_and_bonus_attack();
    v.target_physical_penalty_and_bonus_attack();
    v.bonus_element();
    if v.can_attack_weapon_elemental_command() {
        v.calc_hp_damage();
        v.try_alter_magic_statuses(rng);
    }
    Ok(())
}

/// Script 20 (`0020_OriginalMagicAttackScript.cs:20`).
fn original_magic_attack(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    v.original_magic_params(rng);
    v.caster_enemy_trance_bonus_attack();
    v.caster_penalty_mini();
    v.target_penalty_shell_attack();
    v.bonus_element();
    if v.can_attack_magic() {
        v.calc_hp_damage();
        v.try_alter_magic_statuses(rng);
    }
    Ok(())
}

/// Script 22 (`0022_LvDirectHPDamageScript.cs`).
fn lv_direct_hp_damage(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    if v.is_target_level_multiple_of_command_rate()? && v.target_can_be_attacked() {
        v.try_direct_hp_damage();
    }
    Ok(())
}

/// Script 25 (`0025_PreciseDirectHPDamageScript.cs`).
fn precise_direct_hp_damage(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    if v.target_check_unsafety_or_miss() && v.target_can_be_attacked() {
        v.try_direct_hp_damage();
    }
    Ok(())
}

/// Script 27 (`0027_DirectHPDamageScript.cs:20`).
fn direct_hp_damage(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if v.target_check_unsafety_or_miss() && v.target_can_be_attacked() {
        v.magic_accuracy();
        v.target_penalty_shell_hit_rate();
        if v.try_magic_hit(rng) {
            v.try_direct_hp_damage();
        }
    }
    Ok(())
}

/// Script 63 (`0063_MagicSwordAttackScript.cs`).
fn magic_sword_attack(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    v.caster_set_low_physical_attack(rng);
    v.set_weapon_power_sum();
    v.caster_enemy_trance_bonus_attack();
    v.target_set_physical_defense();
    v.bonus_element();
    if v.can_attack_magic() {
        v.calc_hp_damage();
    }
    Ok(())
}

/// Script 69, potions (`0069_ItemPotionScript.cs:20`). `RandomInt(0, 100)` is
/// one `GameRandom` range draw, taken only when the item has a status.
fn item_potion(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    let item = v.command.item;
    let status = item.map_or(v.command.ability_status, |i| i.status);
    v.attack = 15;
    v.attack_power = item.map_or(v.command.power, |i| i.power);
    v.defense_power = 0;
    if v.attack_power > 0 {
        v.calc_hp_magic_recovery();
    }
    if status != 0 {
        let status_effect_only = v.attack_power == 0;
        let rate = item.map_or(v.command.hit_rate, |i| i.hit_rate);
        if rng.random_range(0, 100) < rate {
            v.target_try_alter_statuses(status, status_effect_only, rng);
        } else if status_effect_only {
            v.flags.0 |= CalcFlags::MISS;
        }
    }
    Ok(())
}

/// Script 73 (`0073_ItemCureStatusScript.cs`).
fn item_cure_status(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    v.try_remove_item_statuses();
    Ok(())
}

/// Script 42, Throw (`0042_ThrowScript.cs:19`). `Command.Weapon.Power` is the
/// thrown item's weapon power, carried in `item_weapon_power`.
fn throw(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if v.target_try_kill_frozen() {
        return Ok(());
    }
    v.caster_set_low_physical_attack(rng);
    v.target_set_physical_defense();
    let power = v
        .command
        .item_weapon_power
        .ok_or(CalcError::MissingItemWeapon)?;
    v.attack_power = power << 1;
    v.caster_physical_penalty_and_bonus_attack();
    v.target_physical_penalty_and_bonus_attack();
    if v.t().has_flight_category() {
        v.modifier_up();
    }
    v.calc_physical_hp_damage();
    Ok(())
}

/// Script 62, Soft (`0062_ItemSoftScript.cs:19`). A Stone-category target is
/// killed (the follow message is UI); anything else loses the item's statuses.
fn item_soft(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    if !v.t().has_stone_category() {
        v.try_remove_item_statuses();
        return Ok(());
    }
    if v.target_can_be_attacked() {
        v.t_mut().kill();
    }
    Ok(())
}

/// Script 70, Ether (`0070_ItemEtherScript.cs:20`).
fn item_ether(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    v.attack = 15;
    v.attack_power = v.command.item.map_or(v.command.power, |i| i.power);
    v.defense_power = 0;
    v.calc_mp_magic_recovery();
    Ok(())
}

/// Script 71, Elixir (`0071_ItemElixirScript.cs:19`): sets HP and MP directly,
/// no `CalcResult` figures.
fn item_elixir(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    if !v.target_can_be_attacked() {
        return Ok(());
    }
    if v.t().is_zombie() {
        v.t_mut().cur_mp = 0;
        v.t_mut().kill();
    } else {
        let max_hp = v.t().max_hp;
        v.t_mut().set_current_hp(max_hp);
        let max_mp = v.t().max_mp;
        v.t_mut().cur_mp = max_mp;
    }
    Ok(())
}

/// Script 72, Phoenix Down and Pinion (`0072_ItemPhoenixScript.cs:19`).
/// `GameRandom.Next8()` is `random8`. A zombie that does not guard sets HP to
/// `Next8 % 10` and dies on 0 (the comparison reads the assigned value, not
/// the clamped HP).
fn item_phoenix(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if !v.target_can_be_revived() {
        return Ok(());
    }
    if v.t().is_zombie() {
        if v.target_check_unsafety_or_guard() {
            let hp = (rng.random8() % 10) as u32;
            v.t_mut().set_current_hp(hp);
            if hp == 0 {
                v.t_mut().kill();
            }
        }
    } else if v.target_check_is_player() {
        if v.t().is_under_any_status(status::DEATH) {
            let hp = (1 + rng.random8() % 10) as u32;
            v.t_mut().set_current_hp(hp);
        }
        v.try_remove_item_statuses();
    }
    Ok(())
}

/// `Command.Item.Power` or `Command.Power` (`ItemId != NoItem ? ...`).
fn item_or_command_power(v: &Calculator<'_>) -> i32 {
    v.command.item.map_or(v.command.power, |i| i.power)
}

/// Script 74, gems (`0074_ItemGemScript.cs:19`): power times the remaining
/// count plus one.
fn item_gem(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    v.target_calc.flags.0 |= UnitFlags::HP_ALTERATION | UnitFlags::HP_RECOVERY;
    v.target_calc.hp_damage =
        item_or_command_power(v).wrapping_mul((v.command.item_count as i32).wrapping_add(1));
    Ok(())
}

/// Script 75, Dead Pepper (`0075_DeadPepperScript.cs:19`).
fn dead_pepper(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    v.target_calc.flags.0 |= UnitFlags::HP_ALTERATION;
    v.target_calc.hp_damage =
        item_or_command_power(v).wrapping_mul((v.command.item_count as i32).wrapping_add(1));
    Ok(())
}

/// Script 76, Tent (`0076_TentScript.cs:19`): half of max HP and MP, and one
/// `Next16() % 100` draw against the item's rate that adds the item's statuses
/// when the rate is lower.
fn tent(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    v.target_calc.flags.0 |= UnitFlags::HP_ALTERATION
        | UnitFlags::HP_RECOVERY
        | UnitFlags::MP_ALTERATION
        | UnitFlags::MP_RECOVERY;
    v.target_calc.hp_damage = (v.t().max_hp >> 1) as i32;
    v.target_calc.mp_damage = (v.t().max_mp >> 1) as i32;
    let item = v.command.item.unwrap_or_default();
    if item.hit_rate < rng.random16() % 100 {
        v.target_try_alter_statuses(item.status, false, rng);
    }
    Ok(())
}

/// Script 96, Trance Full (`0096_TranceFullScript.cs:21`): a miss without
/// a trance gauge (`HasTrance` = `bi.t_gauge != 0`), else a full gauge and
/// the Trance status. The Evil Forest cage (`EVT_BATTLE_EF_E006`) uses it to
/// start the Trance tutorial.
fn trance_full(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if !v.t().has_trance_gauge {
        v.flags.0 |= CalcFlags::MISS;
        return Ok(());
    }
    v.t_mut().trance = u8::MAX;
    let ctx = status::StatusCtx {
        table: v.table,
        env: &v.env.status,
    };
    status::alter_status(&mut v.units[v.target], 14, false, &ctx, rng);
    Ok(())
}

/// Script 77, Dark Matter (`0077_DarkMatterScript.cs:18`).
fn dark_matter(v: &mut Calculator<'_>) -> Result<(), CalcError> {
    if v.target_can_be_attacked() {
        v.target_calc.flags.0 |= UnitFlags::HP_ALTERATION;
        v.target_calc.hp_damage = 9999;
    }
    Ok(())
}

/// Script 103 (`0103_MagicApplyPositiveStatusScript.cs`).
fn magic_apply_positive_status(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    v.try_alter_command_statuses(rng);
    Ok(())
}

/// Script 109, Mini (`0109_MiniScript.cs`).
fn mini(v: &mut Calculator<'_>, rng: Rng<'_>) -> Result<(), CalcError> {
    if v.t().is_under_any_status(status::MINI) {
        v.try_alter_command_statuses(rng);
        return Ok(());
    }
    v.magic_accuracy();
    v.target_penalty_shell_hit_rate();
    v.penalty_command_divided_hit_rate();
    if v.try_magic_hit(rng) {
        v.try_alter_command_statuses(rng);
    }
    Ok(())
}
