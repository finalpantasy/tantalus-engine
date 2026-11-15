//! New-game player records.
//!
//! Port of Memoria `ff9play.FF9Play_Init` / `FF9Play_New` / `FF9Play_Build`
//! for the values the event engine reads (`B_CURHP`, `B_MAXHP`, `B_CURMP`).
//! Every one of the 12 characters gets a player record at new game, built at
//! level 1 (`FF9PLAY_DEF_LEVEL`) with current HP and MP at their maximum.
//!
//! Inputs are the reference tables `BaseStats.csv` (`Comment;Id;Speed;
//! Strength;Magic;Spirit;Gems`) and `Leveling.csv` (`Experience;BonusHP;
//! BonusMP`, one row per level), read by the caller at run time; this crate
//! embeds no game data. Formulas (`ff9level.cs`):
//! `str = min(99, Strength + lv*3/10)`, `mgc = min(99, Magic + lv*3/10)`,
//! `max_hp = min(9999, BonusHP[lv-1] * str / 50)`,
//! `max_mp = min(999, BonusMP[lv-1] * mgc / 100)`. Level-up bonuses are zero
//! at new game, and level-1 records have no supporting abilities, so the
//! equipment and ability passes of `FF9Play_Update` do not change HP or MP.

use std::collections::BTreeMap;

use crate::field::CharacterStats;

/// `ff9play.FF9PLAY_DEF_LEVEL`.
pub const NEW_GAME_LEVEL: usize = 1;
/// `ff9play.FF9PLAY_HP_MAX` / `FF9PLAY_MP_MAX`.
pub const HP_LIMIT: u32 = 9999;
pub const MP_LIMIT: u32 = 999;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseStats {
    pub speed: u8,
    pub strength: u8,
    pub magic: u8,
    pub spirit: u8,
    pub gems: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelRow {
    pub experience: u32,
    pub bonus_hp: u16,
    pub bonus_mp: u16,
}

/// Data rows of a Memoria CSV: `#` starts a comment, fields split on `;`.
fn rows(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .map(|line| line.trim_start_matches('\u{feff}'))
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.split(';').map(str::trim).collect())
}

fn field<T: std::str::FromStr>(row: &[&str], index: usize, file: &str) -> Result<T, String> {
    let text = row
        .get(index)
        .ok_or_else(|| format!("{file}: row {row:?} has no column {index}"))?;
    text.parse()
        .map_err(|_| format!("{file}: column {index} `{text}` is not a number"))
}

pub fn parse_base_stats(text: &str) -> Result<BTreeMap<i32, BaseStats>, String> {
    let mut stats = BTreeMap::new();
    for row in rows(text) {
        // The leading Comment column holds the character name.
        let id: i32 = field(&row, 1, "BaseStats.csv")?;
        stats.insert(
            id,
            BaseStats {
                speed: field(&row, 2, "BaseStats.csv")?,
                strength: field(&row, 3, "BaseStats.csv")?,
                magic: field(&row, 4, "BaseStats.csv")?,
                spirit: field(&row, 5, "BaseStats.csv")?,
                gems: field(&row, 6, "BaseStats.csv")?,
            },
        );
    }
    if (0..12).any(|id| !stats.contains_key(&id)) {
        return Err("BaseStats.csv must define characters 0 to 11".into());
    }
    Ok(stats)
}

pub fn parse_leveling(text: &str) -> Result<Vec<LevelRow>, String> {
    let levels = rows(text)
        .map(|row| {
            Ok(LevelRow {
                experience: field(&row, 0, "Leveling.csv")?,
                bonus_hp: field(&row, 1, "Leveling.csv")?,
                bonus_mp: field(&row, 2, "Leveling.csv")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if levels.len() < 99 {
        return Err(format!(
            "Leveling.csv must define 99 levels, found {}",
            levels.len()
        ));
    }
    Ok(levels)
}

/// `FF9Play_Build(play, lv, init: true, lvup: false)` HP and MP for a
/// character with no level-up bonus.
pub fn build_stats(base: BaseStats, levels: &[LevelRow], level: usize) -> Option<CharacterStats> {
    let row = levels.get(level.checked_sub(1)?)?;
    let lv = level as u32;
    let strength = (u32::from(base.strength) + lv * 3 / 10).min(99);
    let magic = (u32::from(base.magic) + lv * 3 / 10).min(99);
    let max_hp = (u32::from(row.bonus_hp) * strength / 50).min(HP_LIMIT) as i32;
    let max_mp = (u32::from(row.bonus_mp) * magic / 100).min(MP_LIMIT) as i32;
    Some(CharacterStats {
        current_hp: max_hp,
        max_hp,
        current_mp: max_mp,
        max_mp,
    })
}

/// Player records for every character after `FF9Play_Init`.
pub fn new_game_character_stats(
    base_stats: &BTreeMap<i32, BaseStats>,
    levels: &[LevelRow],
) -> BTreeMap<i32, CharacterStats> {
    base_stats
        .iter()
        .filter_map(|(id, base)| Some((*id, build_stats(*base, levels, NEW_GAME_LEVEL)?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic rows in the reference layout (not game data).
    fn base_csv() -> String {
        let mut text = String::from("# Comment;Id;Speed;Strength;Magic;Spirit;Gems\n");
        for id in 0..12 {
            text.push_str(&format!("Name{id};{id};10;{};{};10;5\n", 20 + id, 10 + id));
        }
        text
    }

    fn level_csv() -> String {
        let mut text = String::from("\u{feff}# Experience;BonusHP;BonusMP\n");
        for level in 1..=99 {
            text.push_str(&format!(
                "{};{};{};# Level {level}\n",
                level * 10,
                100 + level,
                50
            ));
        }
        text
    }

    #[test]
    fn parses_reference_layout_with_comments_and_bom() {
        let base = parse_base_stats(&base_csv()).unwrap();
        assert_eq!(base[&3].strength, 23);
        assert_eq!(base[&3].magic, 13);
        let levels = parse_leveling(&level_csv()).unwrap();
        assert_eq!(levels.len(), 99);
        assert_eq!(
            levels[0],
            LevelRow {
                experience: 10,
                bonus_hp: 101,
                bonus_mp: 50
            }
        );
    }

    #[test]
    fn level_one_records_use_the_reference_formulas_and_start_full() {
        let base = parse_base_stats(&base_csv()).unwrap();
        let levels = parse_leveling(&level_csv()).unwrap();
        let stats = new_game_character_stats(&base, &levels);
        assert_eq!(stats.len(), 12);
        // Character 1: str 21 -> 101 * 21 / 50 = 42; mgc 11 -> 50 * 11 / 100 = 5.
        assert_eq!(
            stats[&1],
            CharacterStats {
                current_hp: 42,
                max_hp: 42,
                current_mp: 5,
                max_mp: 5
            }
        );
        // Level 10 adds lv*3/10 = 3 to strength and magic.
        let high = build_stats(base[&1], &levels, 10).unwrap();
        assert_eq!(high.max_hp, 110 * 24 / 50);
        assert!(build_stats(base[&1], &levels, 0).is_none());
    }

    #[test]
    fn missing_characters_or_levels_are_errors() {
        assert!(parse_base_stats("Zidane;0;1;2;3;4;5\n").is_err());
        assert!(parse_leveling("0;250;200\n").is_err());
        assert!(parse_base_stats("Zidane;x;1;2;3;4;5\n").is_err());
    }
}
