//! The Abilities screen of the field main menu.
//!
//! Two tabs per character, as in the reference (`AbilityUI`): the active
//! abilities (learned, or granted by what is equipped, with the AP progress
//! toward learning them) and the support abilities, switched on with magic
//! stones. The rules are the legacy `menu::ability` core; this file loads
//! its tables from the Memoria CSVs and draws rows.
//!
//! Using an ability on the field covers Magic Recovery (script 10: Cure,
//! Cura, Curaga) only; the other menu-usable abilities (status cures,
//! revives, drains) need status and KO tracking the field world does not
//! have and answer "cannot be used here".

use std::collections::BTreeMap;
use std::path::Path;

use tantalus_game::FieldWorld;
use tantalus_legacy::battle::rewards::PaEntry;
use tantalus_legacy::menu::ability::{
    AbilityContext, AbilityType, ActionInfo, ability_lists, active_index, ap_of, check_aa_type,
    check_sa_type, equipped_abilities, has_ap, is_support, max_ap_of, max_capacity, mp_cost,
    play_update, support_index, support_row_display, toggle_support,
};
use tantalus_legacy::menu::{MenuCharacter, MenuConfig, SupportRow, SupportTable};
use tantalus_legacy::shop::stats;

use crate::field_menu::{MenuFrame, MenuText, ROW, Tint};
use crate::play::{character_name, pad};

const LIST_ROWS: usize = 9;

/// Ability tables, loaded once.
#[derive(Debug, Clone, Default)]
pub struct AbilityTables {
    /// Character id -> learnable abilities with their AP goals
    /// (`Characters/Abilities/<preset>.csv`).
    pub learnable: BTreeMap<i32, Vec<PaEntry>>,
    /// Gem costs (`AbilityGems.csv`).
    pub support: SupportTable,
    /// `Actions.csv` rows by ability id: MP and menu flag bits.
    pub actions: Vec<ActionInfo>,
    /// Ability id -> power, for the abilities whose field script is Magic
    /// Recovery.
    pub heal_power: BTreeMap<i32, i32>,
    pub active_names: BTreeMap<i32, String>,
    pub support_names: BTreeMap<i32, String>,
}

/// `CharacterPresetId` names, which are the ability file names.
const PRESET_FILES: [&str; 16] = [
    "Zidane", "Vivi", "Garnet", "Steiner", "Freya", "Quina", "Eiko", "Amarant", "Cinna1", "Cinna2",
    "Marcus1", "Marcus2", "Blank1", "Blank2", "Beatrix1", "Beatrix2",
];

fn rows(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .map(|line| line.trim_start_matches('\u{feff}'))
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| line.split(';').collect())
}

fn int(row: &[&str], index: usize) -> Option<i32> {
    row.get(index)?.trim().parse().ok()
}

/// `0`, `AA:x` (x) or `SA:x` (192 + x).
fn ability_id(text: &str) -> Option<i32> {
    let text = text.trim();
    if let Some(id) = text.strip_prefix("AA:") {
        return id.trim().parse().ok();
    }
    if let Some(id) = text.strip_prefix("SA:") {
        return id.trim().parse::<i32>().ok().map(|id| id + 192);
    }
    text.parse().ok()
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

impl AbilityTables {
    /// Reads the tables under the Memoria data root.
    pub fn load(
        data: &Path,
        active_names: BTreeMap<i32, String>,
        support_names: BTreeMap<i32, String>,
    ) -> Result<Self, String> {
        let characters = data.join("Characters");
        let mut presets = BTreeMap::new();
        for row in rows(&read(&characters.join("CharacterParameters.csv"))?) {
            if let (Some(id), Some(preset)) = (int(&row, 0), int(&row, 4)) {
                presets.insert(id, preset);
            }
        }
        let mut by_preset: BTreeMap<i32, Vec<PaEntry>> = BTreeMap::new();
        for (preset, name) in PRESET_FILES.iter().enumerate() {
            let Ok(text) = read(&characters.join("Abilities").join(format!("{name}.csv"))) else {
                continue;
            };
            let list = rows(&text)
                .filter_map(|row| {
                    Some(PaEntry {
                        id: ability_id(row.first()?)?,
                        max_ap: int(&row, 1)?,
                    })
                })
                .collect();
            by_preset.insert(preset as i32, list);
        }
        let learnable = presets
            .iter()
            .filter_map(|(character, preset)| Some((*character, by_preset.get(preset)?.clone())))
            .collect();
        // Comment;Id;Gems;BoostedVersion(s)
        let mut support = SupportTable::default();
        for row in rows(&read(&characters.join("Abilities/AbilityGems.csv"))?) {
            let Some(id) = int(&row, 1).and_then(|id| usize::try_from(id).ok()) else {
                continue;
            };
            if support.rows.len() <= id {
                support.rows.resize(id + 1, SupportRow::default());
            }
            support.rows[id] = SupportRow {
                gems: int(&row, 2).unwrap_or(0),
                boosted: row
                    .get(3)
                    .map(|list| {
                        list.split(',')
                            .filter_map(|v| v.trim().parse().ok())
                            .collect()
                    })
                    .unwrap_or_default(),
            };
        }
        // Comment;Id;Display;Targets;DefaultAlly;ForDead;DefaultDead;DefaultCamera;
        // AnimationId;AnimationId2;ScriptId;Power;Elements;Rate;Category;AddStatus;MP;Type
        let mut actions = Vec::new();
        let mut heal_power = BTreeMap::new();
        for row in rows(&read(&data.join("Battle/Actions.csv"))?) {
            let Some(id) = int(&row, 1).and_then(|id| usize::try_from(id).ok()) else {
                continue;
            };
            if actions.len() <= id {
                actions.resize(id + 1, ActionInfo::default());
            }
            actions[id] = ActionInfo {
                mp: int(&row, 16).unwrap_or(0),
                kind: int(&row, 17).unwrap_or(0) as u8,
            };
            if int(&row, 10) == Some(10) {
                heal_power.insert(id as i32, int(&row, 11).unwrap_or(0));
            }
        }
        Ok(Self {
            learnable,
            support,
            actions,
            heal_power,
            active_names,
            support_names,
        })
    }

    fn name(&self, id: i32) -> String {
        if is_support(id) {
            let index = support_index(id).unwrap_or(0);
            self.support_names
                .get(&index)
                .cloned()
                .unwrap_or_else(|| format!("Support {index}"))
        } else {
            self.active_names
                .get(&active_index(id).unwrap_or(0))
                .cloned()
                .unwrap_or_else(|| format!("Ability {id}"))
        }
    }
}

/// The ability screen's position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityState {
    pub member: i32,
    /// Support tab (`true`) or active tab.
    pub support: bool,
    pub cursor: usize,
    /// Choosing who an active ability goes to: (ability, member cursor).
    pub target: Option<(i32, usize)>,
}

impl AbilityState {
    pub fn new(member: i32) -> Self {
        Self {
            member,
            support: false,
            cursor: 0,
            target: None,
        }
    }
}

/// The character as the ability rules see them, from the world.
pub fn menu_character(
    world: &FieldWorld,
    tables: &crate::field_menu::MenuTables,
    member: i32,
) -> MenuCharacter {
    let mut ch = tables.character(world, member);
    ch.learnable = tables
        .abilities
        .learnable
        .get(&member)
        .cloned()
        .unwrap_or_default();
    let progress = world.ability_pa.get(&member);
    let learned = world.learned_abilities.get(&member);
    ch.pa = ch
        .learnable
        .iter()
        .map(|entry| {
            if learned.is_some_and(|set| set.contains(&entry.id)) {
                entry.max_ap
            } else {
                progress
                    .and_then(|map| map.get(&entry.id))
                    .copied()
                    .unwrap_or(0)
            }
        })
        .collect();
    ch.enabled = world
        .support_enabled
        .get(&member)
        .cloned()
        .unwrap_or_default();
    let level = tables.level(world.character_exp.get(&member).copied().unwrap_or(0)) as i32;
    let gems = tables.base.get(&member).map_or(0, |base| base.gems as i32);
    ch.max_capa = max_capacity(gems, level, 0);
    ch.basis_capa = ch.max_capa;
    if let Some(stat) = world.character_stats.get(&member) {
        ch.cur_hp = stat.current_hp.max(0) as u32;
        ch.cur_mp = stat.current_mp.max(0) as u32;
    }
    play_update(&mut ch, &tables.abilities.support, MenuConfig::default());
    ch
}

/// Rows of one tab: (ability id, type).
fn tab_rows(
    ch: &MenuCharacter,
    tables: &crate::field_menu::MenuTables,
    support: bool,
) -> Vec<(i32, AbilityType)> {
    let ctx = AbilityContext::default();
    let eqp = equipped_abilities(&tables.db, &ch.equip);
    let lists = ability_lists(ch, &tables.abilities.support);
    let table = &tables.abilities;
    if support {
        lists
            .support
            .into_iter()
            .map(|id| (id, check_sa_type(ch, &table.support, &ctx, &eqp, id)))
            .filter(|(_, ty)| *ty != AbilityType::NoDraw)
            .collect()
    } else {
        lists
            .active
            .into_iter()
            .map(|id| (id, check_aa_type(ch, &ctx, &eqp, &table.actions, id)))
            .filter(|(_, ty)| *ty != AbilityType::NoDraw)
            .collect()
    }
}

/// What a press did to the screen.
pub enum Exit {
    Stay,
    /// Cancel on the tab list.
    Back,
}

impl AbilityState {
    pub fn press(
        &mut self,
        pressed: i32,
        world: &mut FieldWorld,
        tables: &crate::field_menu::MenuTables,
        message: &mut Option<String>,
        seed: u32,
    ) -> Exit {
        let confirm = pressed & (pad::CONFIRM | pad::CIRCLE) != 0;
        let cancel = pressed & (pad::CANCEL | pad::CROSS) != 0;
        let party = world.party.len().min(4);
        if let Some((ability, cursor)) = self.target {
            let cursor = crate::field_menu::step(cursor, party, pressed);
            self.target = Some((ability, cursor));
            if cancel {
                self.target = None;
                *message = None;
            } else if confirm && let Some(target) = world.party.get(cursor).copied() {
                *message = Some(cast(world, tables, self.member, ability, target, seed));
            }
            return Exit::Stay;
        }
        if pressed & (pad::LEFT | pad::RIGHT) != 0 {
            self.support = !self.support;
            self.cursor = 0;
            *message = None;
        }
        let ch = menu_character(world, tables, self.member);
        let list = tab_rows(&ch, tables, self.support);
        self.cursor = crate::field_menu::step(self.cursor, list.len(), pressed);
        if cancel {
            *message = None;
            return Exit::Back;
        }
        if confirm && let Some((id, ty)) = list.get(self.cursor).copied() {
            *message = None;
            if self.support {
                let mut ch = ch;
                let table = &tables.abilities;
                let eqp = equipped_abilities(&tables.db, &ch.equip);
                let ctx = AbilityContext::default();
                match toggle_support(&mut ch, &table.support, &ctx, &eqp, id) {
                    Ok(_) => {
                        world
                            .support_enabled
                            .insert(self.member, ch.enabled.clone());
                    }
                    Err(_) => {
                        *message = Some(if ty == AbilityType::CantSpell {
                            "Not enough magic stones.".into()
                        } else {
                            "That cannot be changed.".into()
                        });
                    }
                }
            } else if ty == AbilityType::Enable && tables.abilities.heal_power.contains_key(&id) {
                self.target = Some((id, 0));
            } else {
                *message = Some(if ty == AbilityType::CantSpell {
                    "It cannot be used now.".into()
                } else {
                    "It cannot be used here.".into()
                });
            }
        }
        Exit::Stay
    }
}

/// Magic Recovery on one member: `power * (magic + random % (1 + (level +
/// magic >> 3)))` HP (`FieldCalculator.SetupSpellHeal`, `HealHp`), costing
/// the caster the ability's MP.
fn cast(
    world: &mut FieldWorld,
    tables: &crate::field_menu::MenuTables,
    caster: i32,
    ability: i32,
    target: i32,
    seed: u32,
) -> String {
    let ch = menu_character(world, tables, caster);
    let Some(info) = active_index(ability)
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| tables.abilities.actions.get(i).copied())
    else {
        return "It cannot be used here.".into();
    };
    let cost = mp_cost(&info, false);
    let power = tables
        .abilities
        .heal_power
        .get(&ability)
        .copied()
        .unwrap_or(0);
    let Some(hurt) = world.character_stats.get(&target).copied() else {
        return "It cannot be used here.".into();
    };
    if hurt.current_hp <= 0 || hurt.current_hp >= hurt.max_hp {
        return "It had no effect.".into();
    }
    if (ch.cur_mp as i32) < cost {
        return "Not enough MP.".into();
    }
    let level = tables.level(world.character_exp.get(&caster).copied().unwrap_or(0)) as i32;
    let magic = stats(&tables.db, ch.basis, &ch.equip).mgc;
    let attack = magic + (seed as i32) % (1 + ((level + magic) >> 3));
    let healed = (power * attack).min(hurt.max_hp - hurt.current_hp);
    if let Some(stat) = world.character_stats.get_mut(&target) {
        stat.current_hp += healed;
    }
    if let Some(stat) = world.character_stats.get_mut(&caster) {
        stat.current_mp = (stat.current_mp - cost).max(0);
    }
    format!("{} recovered {healed} HP.", character_name(target))
}

fn text(x: f32, row: f32, value: impl Into<String>, tint: Tint) -> MenuText {
    MenuText {
        x,
        y: row * ROW,
        text: value.into(),
        tint,
        right: false,
    }
}

fn right(x: f32, row: f32, value: impl Into<String>, tint: Tint) -> MenuText {
    MenuText {
        right: true,
        ..text(x, row, value, tint)
    }
}

/// Draws the screen into `frame`.
pub fn frame(
    state: &AbilityState,
    frame: &mut MenuFrame,
    world: &FieldWorld,
    tables: &crate::field_menu::MenuTables,
) {
    let ch = menu_character(world, tables, state.member);
    let list = tab_rows(&ch, tables, state.support);
    let table = &tables.abilities;
    let top = state.cursor.saturating_sub(LIST_ROWS - 1);
    let mut texts = Vec::new();
    for (row, (id, ty)) in list.iter().skip(top).take(LIST_ROWS).enumerate() {
        let r = row as f32;
        let ap = ap_of(&ch, *id);
        let max = max_ap_of(&ch, *id);
        let learning = has_ap(&ch) && ap >= 0 && ap < max;
        if state.support {
            let (shown, cost) =
                support_row_display(&ch, &table.support, support_index(*id).unwrap_or(0), *ty);
            let _ = shown;
            let tint = match ty {
                AbilityType::Selected => Tint::Green,
                AbilityType::CantSpell => Tint::Dim,
                AbilityType::CantDisable => Tint::Yellow,
                _ => Tint::White,
            };
            texts.push(text(12.0, r, table.name(*id), tint));
            if learning {
                texts.push(right(142.0, r, format!("{ap}/{max}"), Tint::Yellow));
            } else {
                texts.push(right(142.0, r, cost.to_string(), tint));
            }
        } else {
            let tint = if *ty == AbilityType::Enable {
                Tint::White
            } else {
                Tint::Dim
            };
            texts.push(text(12.0, r, table.name(*id), tint));
            if learning {
                texts.push(right(142.0, r, format!("{ap}/{max}"), Tint::Yellow));
            } else if let Some(info) = active_index(*id)
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| table.actions.get(i))
            {
                texts.push(right(142.0, r, mp_cost(info, false).to_string(), tint));
            }
        }
    }
    if list.is_empty() {
        texts.push(text(12.0, 0.0, "Nothing here", Tint::Dim));
    }
    let tab = |name: &str, on: bool| (name.to_string(), if on { Tint::Yellow } else { Tint::Dim });
    let (magic, magic_tint) = tab("Magic", !state.support);
    let (sup, sup_tint) = tab("Support", state.support);
    frame.windows.push(crate::field_menu::window(
        8.0,
        8.0,
        150.0,
        1.0,
        vec![
            text(8.0, 0.0, magic, magic_tint),
            text(60.0, 0.0, sup, sup_tint),
            right(142.0, 0.0, character_name(state.member), Tint::White),
        ],
    ));
    frame.windows.push(crate::field_menu::window(
        8.0,
        32.0,
        150.0,
        LIST_ROWS as f32,
        texts,
    ));
    frame.windows.push(crate::field_menu::window(
        8.0,
        166.0,
        150.0,
        1.0,
        vec![
            text(8.0, 0.0, "Stones", Tint::Yellow),
            right(
                142.0,
                0.0,
                format!("{}/{}", ch.cur_capa, ch.max_capa),
                Tint::White,
            ),
        ],
    ));
    frame.windows.push(crate::field_menu::party_window(
        world, tables, 164.0, 148.0, 4,
    ));
    frame.pointer = Some(match state.target {
        Some((_, member)) => (168.0, 8.0 + 3.0 + (member as f32 * 3.0 + 0.5) * ROW),
        None => (10.0, 32.0 + 3.0 + ((state.cursor - top) as f32 + 0.5) * ROW),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field_menu::MenuTables;
    use tantalus_game::field::world::CharacterStats;
    use tantalus_game::new_game::BaseStats;

    fn tables() -> MenuTables {
        let mut tables = MenuTables {
            exp: vec![0, 16, 47],
            ..MenuTables::default()
        };
        tables.base.insert(
            0,
            BaseStats {
                speed: 20,
                strength: 20,
                magic: 18,
                spirit: 20,
                gems: 10,
            },
        );
        let table = &mut tables.abilities;
        // Ability 1 (Cure) and support ability 1 (id 193, 6 stones).
        table.learnable.insert(
            0,
            vec![
                PaEntry { id: 1, max_ap: 30 },
                PaEntry {
                    id: 193,
                    max_ap: 20,
                },
            ],
        );
        table.actions = vec![ActionInfo::default(), ActionInfo { mp: 6, kind: 1 }];
        table.heal_power.insert(1, 16);
        table.support.rows = vec![
            SupportRow::default(),
            SupportRow {
                gems: 6,
                boosted: vec![],
            },
        ];
        tables
    }

    fn world() -> FieldWorld {
        let mut world = FieldWorld::new();
        world.party = vec![0];
        world.character_stats.insert(
            0,
            CharacterStats {
                current_hp: 10,
                max_hp: 100,
                current_mp: 20,
                max_mp: 20,
            },
        );
        world.apply_ap(0, 1, 30, 30);
        world.apply_ap(0, 193, 20, 20);
        world
    }

    #[test]
    fn stat_growth_follows_the_level_table() {
        let (tables, mut world) = (tables(), world());
        // Level 3 (47 EXP): speed +0, strength +0, magic +0, spirit +0.
        world.character_exp.insert(0, 47);
        let at_3 = tables.character(&world, 0).basis;
        assert_eq!(at_3, [20, 20, 18, 20]);
        // 99 EXP is still level 3 in this short table; use a long one.
        let mut long = tables;
        long.exp = (0..99).map(|level| level * 100).collect();
        world.character_exp.insert(0, 9800);
        // Level 99: speed +9, strength +29, magic +29, spirit +14.
        assert_eq!(long.character(&world, 0).basis, [29, 49, 47, 34]);
    }

    #[test]
    fn cure_heals_and_costs_mp() {
        let (tables, mut world) = (tables(), world());
        let mut state = AbilityState::new(0);
        let mut message = None;
        // Confirm Cure, then confirm the first member.
        state.press(pad::CONFIRM, &mut world, &tables, &mut message, 3);
        assert!(state.target.is_some());
        state.press(pad::CONFIRM, &mut world, &tables, &mut message, 3);
        let stat = world.character_stats[&0];
        assert_eq!(stat.current_mp, 14);
        assert!(stat.current_hp > 10);
        assert!(message.unwrap().contains("recovered"));
    }

    #[test]
    fn a_support_ability_takes_and_returns_stones() {
        let (tables, mut world) = (tables(), world());
        let mut state = AbilityState::new(0);
        let mut message = None;
        state.press(pad::RIGHT, &mut world, &tables, &mut message, 0);
        assert!(state.support);
        state.press(pad::CONFIRM, &mut world, &tables, &mut message, 0);
        assert_eq!(world.support_enabled[&0], vec![1]);
        let ch = menu_character(&world, &tables, 0);
        assert_eq!((ch.cur_capa, ch.max_capa), (4, 10));
        state.press(pad::CONFIRM, &mut world, &tables, &mut message, 0);
        assert!(world.support_enabled[&0].is_empty());
        assert_eq!(menu_character(&world, &tables, 0).cur_capa, 10);
    }
}
