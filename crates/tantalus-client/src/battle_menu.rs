//! The battle command menu: when a party member's ATB gauge is full, the
//! pad picks a command (Attack, the member's ability command, Defend, Item),
//! a list entry for abilities and items, then a target, and the choice goes
//! to the battle as a [`BattleCommand`].
//!
//! Follows `BattleHUD` (`Memoria/Assembly-CSharp/Global/BattleHUD*.cs`) in
//! shape: `CommandPanel` -> `AbilityPanel` / `ItemPanel` -> `TargetPanel`,
//! Cancel steps back, and the first ready member (`ReadyQueue` order) is the
//! one asked. Not matched: switching between ready members, the Change and
//! Flee commands, multi-target toggling, the Wait ATB mode (the gauges keep
//! filling while the menu is open, the Active mode).

use tantalus_game::battle::BattleCommand;
use tantalus_tools::field_battle::{LiveFight, MenuAim, MenuEntry};
use tantalus_tools::play::pad;

/// The four command rows.
pub const COMMANDS: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub enum MenuLevel {
    /// The command window, `cursor` on a row (0 Attack, 1 ability command,
    /// 2 Defend, 3 Item).
    Commands,
    /// The ability (`items` false) or item list.
    List { items: bool },
    /// Picking a target for `entry`; `back` is where Cancel returns.
    Target {
        entry: MenuEntry,
        back: Box<MenuLevel>,
    },
}

/// The menu's state between frames.
#[derive(Debug, Clone, PartialEq)]
pub struct BattleMenu {
    /// The party member being asked (machine unit index).
    pub unit: Option<usize>,
    pub level: MenuLevel,
    pub command: usize,
    pub list: usize,
    pub target: usize,
}

impl Default for BattleMenu {
    fn default() -> Self {
        Self {
            unit: None,
            level: MenuLevel::Commands,
            command: 0,
            list: 0,
            target: 0,
        }
    }
}

/// Units `entry` can aim at, in screen order.
pub fn targets(fight: &LiveFight, unit: usize, entry: &MenuEntry) -> Vec<usize> {
    let machine = fight.machine();
    let party = fight.party_len();
    let alive = |index: usize| {
        machine.sched.get(index).is_some_and(|sched| sched.in_list)
            && (entry.for_dead || machine.units[index].cur_hp > 0)
    };
    match entry.aim {
        MenuAim::Caster => vec![unit],
        MenuAim::Allies => (0..party).filter(|&index| alive(index)).collect(),
        MenuAim::Enemies => (party..machine.units.len())
            .filter(|&index| alive(index) && machine.units[index].cur_hp > 0)
            .collect(),
    }
}

/// Whether `entry` can be picked now (enough MP, an item left).
pub fn usable(fight: &LiveFight, unit: usize, entry: &MenuEntry) -> bool {
    let mp = fight
        .machine()
        .units
        .get(unit)
        .map_or(0, |unit| unit.cur_mp);
    match entry.item {
        Some(item) => fight.item_count(item) > 0,
        None => mp >= entry.mp,
    }
}

impl BattleMenu {
    /// Whether the menu is showing.
    pub fn open(&self) -> bool {
        self.unit.is_some()
    }

    /// One frame of input: `pressed` pad bits (new presses only). Returns
    /// the command once one is chosen.
    pub fn update(&mut self, fight: &LiveFight, pressed: i32) -> Option<BattleCommand> {
        let machine = fight.machine();
        let ready: Vec<usize> = machine
            .ready_players
            .iter()
            .copied()
            .filter(|&unit| unit < fight.party_len() && machine.units[unit].cur_hp > 0)
            .collect();
        if self.unit.is_none_or(|unit| !ready.contains(&unit)) {
            *self = Self {
                unit: ready.first().copied(),
                ..Self::default()
            };
        }
        let unit = self.unit?;
        let menu = fight.menu(unit)?;
        let up = pressed & pad::UP != 0;
        let down = pressed & pad::DOWN != 0;
        let left = pressed & pad::LEFT != 0;
        let right = pressed & pad::RIGHT != 0;
        let confirm = pressed & pad::CONFIRM != 0;
        let cancel = pressed & pad::CANCEL != 0;
        let step = |cursor: usize, count: usize, back: bool, forward: bool| {
            if count == 0 {
                0
            } else if back {
                (cursor + count - 1) % count
            } else if forward {
                (cursor + 1) % count
            } else {
                cursor.min(count - 1)
            }
        };
        match self.level.clone() {
            MenuLevel::Commands => {
                self.command = step(self.command, COMMANDS, up, down);
                if !confirm {
                    return None;
                }
                match self.command {
                    0 => self.aim(fight, unit, menu.attack.clone(), MenuLevel::Commands),
                    1 if !menu.abilities.is_empty() => {
                        self.level = MenuLevel::List { items: false };
                        self.list = 0;
                    }
                    2 => self.aim(fight, unit, menu.defend.clone(), MenuLevel::Commands),
                    3 if !menu.items.is_empty() => {
                        self.level = MenuLevel::List { items: true };
                        self.list = 0;
                    }
                    _ => {}
                }
                None
            }
            MenuLevel::List { items } => {
                let list = if items { &menu.items } else { &menu.abilities };
                self.list = step(self.list, list.len(), up, down);
                if cancel {
                    self.level = MenuLevel::Commands;
                } else if confirm
                    && let Some(entry) = list.get(self.list)
                    && usable(fight, unit, entry)
                {
                    let back = self.level.clone();
                    self.aim(fight, unit, entry.clone(), back);
                }
                None
            }
            MenuLevel::Target { entry, back } => {
                let candidates = targets(fight, unit, &entry);
                self.target = step(self.target, candidates.len(), up || left, down || right);
                if cancel {
                    self.level = *back;
                    return None;
                }
                if !confirm {
                    return None;
                }
                let target = candidates.get(self.target).copied()?;
                let command = BattleCommand {
                    actor: fight.entity(unit)?,
                    action: entry.action.clone(),
                    targets: vec![fight.entity(target)?],
                };
                *self = Self::default();
                Some(command)
            }
        }
    }

    /// Opens the target cursor for `entry`, on the caster for allies and
    /// on the first enemy otherwise (`BattleHUD.SetTargetDefault`).
    fn aim(&mut self, fight: &LiveFight, unit: usize, entry: MenuEntry, back: MenuLevel) {
        let candidates = targets(fight, unit, &entry);
        self.target = candidates
            .iter()
            .position(|&candidate| candidate == unit)
            .unwrap_or(0);
        self.level = MenuLevel::Target {
            entry,
            back: Box::new(back),
        };
    }

    /// The unit under the target cursor.
    pub fn targeted(&self, fight: &LiveFight) -> Option<usize> {
        let MenuLevel::Target { entry, .. } = &self.level else {
            return None;
        };
        targets(fight, self.unit?, entry).get(self.target).copied()
    }
}
