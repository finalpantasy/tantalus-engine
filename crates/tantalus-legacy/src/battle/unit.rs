//! Battle unit state read and written by the formulas.
//!
//! Source of each field: `Global/BTL_DATA.cs` (the `Data.*` members) through
//! the accessor in `Memoria/Battle/Calculator/BattleUnit.cs`.

use std::collections::{BTreeMap, BTreeSet};

/// `ElementSet` restricted to the eight vanilla elements, stored as the
/// `EffectElement` byte (`Memoria/Data/Battle/EffectElement.cs:14`): Fire 1,
/// Cold 2, Thunder 4, Earth 8, Aqua 16, Wind 32, Holy 64, Darkness 128.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Elements(pub u8);

impl Elements {
    pub const NONE: Elements = Elements(0);
    pub const FIRE: u8 = 1;
    pub const COLD: u8 = 2;
    pub const THUNDER: u8 = 4;
    pub const EARTH: u8 = 8;
    pub const AQUA: u8 = 16;
    pub const WIND: u8 = 32;
    pub const HOLY: u8 = 64;
    pub const DARKNESS: u8 = 128;

    /// `ElementSet.CountCommon(a, b)` (`EffectElement.cs:96`): size of the
    /// intersection.
    pub fn count_common(self, other: Elements) -> i32 {
        (self.0 & other.0).count_ones() as i32
    }

    pub fn contains(self, bit: u8) -> bool {
        self.0 & bit != 0
    }
}

/// The parts of `BTL_DATA.weapon` the formulas read.
///
/// * `power` = `weapon.Ref.Power` (`BattleUnit.cs:226`).
/// * `rate` = `weapon.Ref.Rate` (`BattleUnit.cs:225`).
/// * `elements` = `weapon.Ref.Elements` (`BattleUnit.cs:228`).
/// * `category` = `weapon.Category`, `WeaponCategory` flags: ShortRange 1,
///   LongRange 2 (`Memoria/Data/Battle/WeaponCategory.cs`).
/// * `status` = the status set of `add_status[weapon.StatusIndex]`
///   (`BattleUnit.cs:229`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WeaponStats {
    pub power: i32,
    pub rate: i32,
    pub elements: Elements,
    pub category: u8,
    pub status: u64,
}

impl WeaponStats {
    pub const SHORT_RANGE: u8 = 1;
    pub const LONG_RANGE: u8 = 2;
}

/// `CharacterCategory.Female` (`Memoria/Data/Battle/CharacterCategory.cs:9`).
pub const CATEGORY_FEMALE: u8 = 2;

/// `PLAYER.category` at character creation: `CharacterParameter.DefaultCategory`
/// (`ff9play.cs:133`), the `DefaultCategory` column of the shipped
/// `Data/Characters/CharacterParameters.csv`, indexed by `CharacterId`
/// (Zidane 0 .. Beatrix 11). `None` for an id outside that table. Mods and
/// ability features can change `PLAYER.category` afterwards; the caller sets
/// [`BattleUnit::player_category`] to the live value.
pub fn default_player_category(character_id: u8) -> Option<u8> {
    const TABLE: [u8; 12] = [9, 5, 6, 5, 6, 5, 6, 5, 21, 21, 21, 22];
    TABLE.get(usize::from(character_id)).copied()
}

/// `PLAYER.info.serial_no` (`CharacterSerialNumber`, Zidane dagger 0 ..
/// Beatrix 18) from the `BattleParameterFormula` column of the shipped
/// `Data/Characters/CharacterParameters.csv`: `weapon_shape` is the
/// equipped weapon's `GraphicsId` (`ff9item.shape`), `scenario` the
/// scenario counter. `None` for an id outside that table.
pub fn default_serial_number(character_id: u8, weapon_shape: u8, scenario: i32) -> Option<u8> {
    Some(match character_id {
        0 => u8::from(weapon_shape != 1),
        1 => 2,
        2 => match (scenario < 10300, weapon_shape == 7) {
            (true, true) => 4,
            (true, false) => 3,
            (false, true) => 6,
            (false, false) => 5,
        },
        3 => 7,
        4 => 12,
        5 => 9,
        6 => {
            if weapon_shape == 7 {
                11
            } else {
                10
            }
        }
        7 => 13,
        8 => 14,
        9 => 15,
        10 => {
            if (1500..1600).contains(&scenario) {
                17
            } else {
                16
            }
        }
        11 => 18,
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct BattleUnit {
    /// `BTL_DATA.btl_id` (bit mask id).
    pub id: u16,
    /// `bi.player != 0`.
    pub is_player: bool,
    /// `bi.slot_no` for players (`CharacterId`, Steiner = 3); 255 = NONE.
    pub character_id: u8,
    /// `info.serial_no` (`CharacterSerialNumber`) for players, read by
    /// battle scripts as member 66 (`MODEL_TYPE`); 255 = NONE.
    pub serial_no: u8,
    /// `Data.level` (Byte).
    pub level: u8,
    /// `Data.cur.hp` / `Data.max.hp` (raw; with `CustomBattleFlagsMeaning = 0`
    /// `GetLogicalHP` returns the raw value, `btl_para.cs:15`).
    pub cur_hp: u32,
    pub max_hp: u32,
    pub cur_mp: u32,
    pub max_mp: u32,
    /// `Data.cur.at`.
    pub cur_atb: i16,
    /// `Data.elem.str`, `.mgc`, `.dex`, `.wpr` (Will = Spirit).
    pub strength: u8,
    pub magic: u8,
    pub dexterity: u8,
    pub will: u8,
    /// `Data.defence.PhysicalDefence`, `PhysicalEvade`, `MagicalDefence`,
    /// `MagicalEvade` (Int32, `BattleUnit.cs:98-119`).
    pub physical_defence: i32,
    pub physical_evade: i32,
    pub magic_defence: i32,
    pub magic_evade: i32,
    /// `bi.row`: players 0 = back row, 1 = front row
    /// (`FF9UIDataTool.cs:175`); enemies are created with 2
    /// (`btl_init.cs:201`).
    pub row: u8,
    /// `bi.cover != 0`.
    pub covering: bool,
    /// `def_attr.invalid` (guard), `half`, `absorb`, `weak`, `p_up_attr`.
    pub guard_elements: Elements,
    pub half_elements: Elements,
    pub absorb_elements: Elements,
    pub weak_elements: Elements,
    pub bonus_elements: Elements,
    /// `stat.cur`, `stat.permanent`, `stat.invalid` (resist). Bit `n` is
    /// `BattleStatusId` `n` (`Memoria/Data/Battle/BattleStatus.cs`).
    pub status_current: u64,
    pub status_permanent: u64,
    pub status_resist: u64,
    /// `stat.conti` / `stat.opr` (status duration and tick counters).
    pub conti: BTreeMap<u8, i32>,
    pub opr: BTreeMap<u8, i32>,
    /// `stat.partial_resist` (default 0 per status) and
    /// `stat.duration_factor` (default 1 per status), `STAT_INFO.cs:21-22`.
    pub partial_resist: BTreeMap<u8, f32>,
    pub duration_factor: BTreeMap<u8, f32>,
    /// `HasCategory(EnemyCategory.Undead)` / `Flight`; only read for enemies
    /// (`btl_util.CheckEnemyCategory`, `btl_util.cs:306`).
    pub is_undead: bool,
    pub is_flight: bool,
    /// `HasCategory(EnemyCategory.Stone)`; only read for enemies.
    pub is_stone: bool,
    /// `ENEMY.info.die_atk` (`BattleEnemy.AttackOnDeath`): the enemy has a
    /// dying move. Only read for enemies.
    pub attack_on_death: bool,
    /// `PLAYER.category` (`CharacterCategory` bits, `BattleUnit.PlayerCategory`,
    /// `BattleUnit.cs:243`); 0 for enemies. Defaults per character:
    /// [`default_player_category`].
    pub player_category: u8,
    pub weapon: Option<WeaponStats>,
    /// `Data.maxDamageLimit` (players: 9999, `ff9play.cs:42`) and
    /// `maxMpDamageLimit` (players: 999, `ff9play.cs:43`).
    pub max_damage_limit: u32,
    pub max_mp_damage_limit: u32,
    /// `critical_rate_deal_bonus` / `critical_rate_receive_resistance`.
    pub critical_rate_bonus: i16,
    pub critical_rate_resistance: i16,
    /// `Data.trance` and `bi.t_gauge != 0`.
    pub trance: u8,
    pub has_trance_gauge: bool,
    /// `CanUseTheAttackCommand` (`BattleUnit.cs:407`): false only for a
    /// monster-transformed player with no attack motion.
    pub can_use_attack_command: bool,
    /// `btl_para.IsNonDyingVanillaBoss` (`btl_para.cs:354`), battle-map
    /// dependent, supplied by the caller.
    pub non_dying_boss: bool,
    /// `bi.death_f` after `Kill`.
    pub death_flag: bool,
    /// `btl_cmd.CheckSpecificCommand(btl, SysStone)` / `PetrifyCommand` queued.
    pub petrify_command_pending: bool,
    /// Doom and Gradual Petrify counters (`Counter`, default start 10 when no
    /// parameter, `DoomStatusScript.cs:17`, `GradualPetrifyStatusScript.cs:18`).
    pub doom_counter: i32,
    pub gradual_petrify_counter: i32,
    /// Support abilities in effect (`ff9abil.GetEnabledSA`, ids of
    /// `SupportAbility`). Which ones a character has equipped and learned is
    /// party state the caller supplies.
    pub support_abilities: BTreeSet<u8>,
    /// `bi.cover_unit`: the `btl_id` of the unit this unit is covering. Set by
    /// `btl_abil.CheckCoverAbility`; `bi.cover` is `cover_unit != null`, so
    /// [`BattleUnit::covering`] is kept equal to `cover_unit.is_some()`.
    pub cover_unit: Option<u16>,
}

impl BattleUnit {
    /// A neutral unit: level 1, all stats 0, no statuses, limits 9999/999.
    pub fn new(id: u16, is_player: bool) -> Self {
        Self {
            id,
            is_player,
            character_id: 255,
            serial_no: 255,
            level: 1,
            cur_hp: 0,
            max_hp: 0,
            cur_mp: 0,
            max_mp: 0,
            cur_atb: 0,
            strength: 0,
            magic: 0,
            dexterity: 0,
            will: 0,
            physical_defence: 0,
            physical_evade: 0,
            magic_defence: 0,
            magic_evade: 0,
            row: if is_player { 1 } else { 2 },
            covering: false,
            guard_elements: Elements::NONE,
            half_elements: Elements::NONE,
            absorb_elements: Elements::NONE,
            weak_elements: Elements::NONE,
            bonus_elements: Elements::NONE,
            status_current: 0,
            status_permanent: 0,
            status_resist: 0,
            conti: BTreeMap::new(),
            opr: BTreeMap::new(),
            partial_resist: BTreeMap::new(),
            duration_factor: BTreeMap::new(),
            is_undead: false,
            is_flight: false,
            is_stone: false,
            attack_on_death: false,
            player_category: 0,
            weapon: None,
            max_damage_limit: 9999,
            max_mp_damage_limit: 999,
            critical_rate_bonus: 0,
            critical_rate_resistance: 0,
            trance: 0,
            has_trance_gauge: false,
            can_use_attack_command: true,
            non_dying_boss: false,
            death_flag: false,
            petrify_command_pending: false,
            doom_counter: 0,
            gradual_petrify_counter: 0,
            support_abilities: BTreeSet::new(),
            cover_unit: None,
        }
    }

    /// `ff9abil.GetEnabledSA(unit)` contains `sa`.
    pub fn has_support_ability(&self, sa: u8) -> bool {
        self.support_abilities.contains(&sa)
    }

    /// `IsUnderAnyStatus(mask)` (`BattleUnit.cs:482`): `(CurrentStatus & mask) != 0`.
    pub fn is_under_any_status(&self, mask: u64) -> bool {
        self.status_current & mask != 0
    }

    /// `IsPlayer ? slot_no : NONE`.
    pub fn player_index(&self) -> u8 {
        if self.is_player {
            self.character_id
        } else {
            255
        }
    }

    /// `HasCategory(EnemyCategory.Undead)`: false for players
    /// (`btl_util.cs:308`).
    pub fn has_undead_category(&self) -> bool {
        !self.is_player && self.is_undead
    }

    pub fn has_flight_category(&self) -> bool {
        !self.is_player && self.is_flight
    }

    /// `HasCategory(EnemyCategory.Stone)`: false for players.
    pub fn has_stone_category(&self) -> bool {
        !self.is_player && self.is_stone
    }

    /// `HasCategory(CharacterCategory)` (`BattleUnit.cs:494`).
    pub fn has_character_category(&self, category: u8) -> bool {
        self.is_player && self.player_category & category != 0
    }

    /// `IsZombie` (`BattleUnit.cs:325`).
    pub fn is_zombie(&self) -> bool {
        self.has_undead_category() || self.is_under_any_status(crate::battle::status::ZOMBIE)
    }

    /// `IsLevitate` (`BattleUnit.cs:324`).
    pub fn is_levitate(&self) -> bool {
        self.has_flight_category() || self.is_under_any_status(crate::battle::status::FLOAT)
    }

    /// `HasLongRangeWeapon` (`BattleUnit.cs:326`, `HasCategory(WeaponCategory)`
    /// at `:514`).
    pub fn has_long_range_weapon(&self) -> bool {
        self.weapon
            .is_some_and(|w| w.category & WeaponStats::LONG_RANGE != 0)
    }

    /// `WeaponElementEx`.
    pub fn weapon_elements(&self) -> Elements {
        self.weapon.map_or(Elements::NONE, |w| w.elements)
    }

    /// `GetWeaponPower(cmd)` with `CustomBattleFlagsMeaning = 0` and no
    /// monster transform (`BattleUnit.cs:230`).
    pub fn weapon_power(&self) -> i32 {
        self.weapon.map_or(0, |w| w.power)
    }

    /// `CurrentHp = value` (`btl_para.SetLogicalHP`, `btl_para.cs:31`), mode 0
    /// branch: bosses that never die in vanilla keep at least 1 HP.
    pub fn set_current_hp(&mut self, value: u32) {
        let mut hp = value;
        if self.non_dying_boss {
            hp = hp.max(1);
        }
        self.cur_hp = hp;
    }

    /// `BattleUnit.Kill` (`BattleUnit.cs:571`). Does not add the Death status
    /// (that happens in `CheckPointData`).
    pub fn kill(&mut self) {
        self.set_current_hp(0);
        if self.cur_hp > 0 {
            return;
        }
        self.death_flag = true;
    }
}
