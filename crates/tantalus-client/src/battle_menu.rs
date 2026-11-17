//! The battle command menu: when a party member's ATB gauge is full, the
//! pad picks a command (Attack, the member's two ability commands, Item,
//! and Defend and Change beside them), a list entry for abilities and
//! items, then a target, and the choice goes to the battle as a
//! [`BattleCommand`].
//!
//! Follows `BattleHUD` (`Memoria/Assembly-CSharp/Global/battle/BattleHUD/`)
//! in shape: `CommandPanel` -> `AbilityPanel` / `ItemPanel` ->
//! `TargetPanel`, Cancel steps back, the first ready member (`ReadyQueue`
//! order) is asked, and Triangle passes the turn to the next ready member
//! (`OnKeyMenu`, `BattleHUD.Scene.cs:409`). R1 switches a `Many*` ability
//! between one target and the whole side. Not matched: the Wait ATB mode
//! (the gauges keep filling while the menu is open, the Active mode),
//! moving the cursor across sides for `SingleAny`, and holding L1 and R1 to
//! run is handled by the caller (it needs time, not presses).

use tantalus_game::battle::BattleCommand;
use tantalus_tools::field_battle::{CommandKind, LiveFight, MenuAim, MenuEntry, PartyMenu};
use tantalus_tools::play::pad;

/// Command window rows: Attack, ability 1, ability 2, Item, then Defend
/// and Change in the side column.
pub const COMMANDS: usize = 6;
pub const ROW_ATTACK: usize = 0;
pub const ROW_ITEM: usize = 3;
pub const ROW_DEFEND: usize = 4;
pub const ROW_CHANGE: usize = 5;

/// Which list is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ListKind {
    /// The ability list of command row 1 or 2 (index into
    /// [`PartyMenu::commands`]).
    Ability(usize),
    Items,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MenuLevel {
    /// The command window, `command` on a row.
    Commands,
    List(ListKind),
    /// Picking targets for `entry`; `back` is where Cancel returns, `all`
    /// whether the cursor covers the whole side.
    Target {
        entry: MenuEntry,
        back: Box<MenuLevel>,
        all: bool,
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
    /// Config `Cursor: Memory` (`cfg.cursor != 0`): each member's menu opens
    /// on the command and list rows last chosen
    /// (`BattleHUD.TryGetMemorizedCommandObject`, `_abilityCursorMemorize`).
    pub remember: bool,
    /// Last command row per member.
    memo_command: std::collections::BTreeMap<usize, usize>,
    /// Last list row per member and list.
    memo_list: std::collections::BTreeMap<(usize, ListKind), usize>,
}

impl Default for BattleMenu {
    fn default() -> Self {
        Self {
            unit: None,
            level: MenuLevel::Commands,
            command: 0,
            list: 0,
            target: 0,
            remember: false,
            memo_command: std::collections::BTreeMap::new(),
            memo_list: std::collections::BTreeMap::new(),
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

/// Whether `entry` can be picked now (ported, enough MP, an item left).
pub fn usable(fight: &LiveFight, unit: usize, entry: &MenuEntry) -> bool {
    let mp = fight
        .machine()
        .units
        .get(unit)
        .map_or(0, |unit| unit.cur_mp);
    entry.enabled
        && match entry.item {
            Some(item) => fight.item_count(item) > 0,
            None => mp >= entry.mp,
        }
}

/// The list `kind` opens in `menu`.
pub fn list_of(menu: &PartyMenu, kind: ListKind) -> &[MenuEntry] {
    match kind {
        ListKind::Items => &menu.items,
        ListKind::Ability(index) => match menu.commands.get(index).map(|command| &command.kind) {
            Some(CommandKind::List(list)) => list,
            _ => &[],
        },
    }
}

/// Whether command row `row` can be picked.
pub fn row_enabled(menu: &PartyMenu, row: usize) -> bool {
    match row {
        1 | 2 => match menu.commands.get(row - 1).map(|command| &command.kind) {
            Some(CommandKind::Direct(entry)) => entry.enabled,
            Some(CommandKind::List(list)) => !list.is_empty(),
            _ => false,
        },
        ROW_ITEM => !menu.items.is_empty(),
        ROW_CHANGE => menu.change.enabled,
        _ => true,
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
            self.reset_to(ready.first().copied());
        }
        let unit = self.unit?;
        // Triangle: the next ready member takes the menu
        // (`OnKeyMenu` rotates `ReadyQueue`).
        if pressed & pad::TRIANGLE != 0
            && self.level == MenuLevel::Commands
            && ready.len() > 1
            && let Some(position) = ready.iter().position(|&ready| ready == unit)
        {
            self.reset_to(Some(ready[(position + 1) % ready.len()]));
            return None;
        }
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
                self.command = match self.command {
                    row @ 0..=3 if up || down => step(row, 4, up, down),
                    row @ 0..=3 if right => {
                        if row <= 1 {
                            ROW_DEFEND
                        } else {
                            ROW_CHANGE
                        }
                    }
                    ROW_DEFEND if up || down => ROW_CHANGE,
                    ROW_CHANGE if up || down => ROW_DEFEND,
                    ROW_DEFEND if left => ROW_ATTACK,
                    ROW_CHANGE if left => ROW_ITEM,
                    row => row.min(COMMANDS - 1),
                };
                if !confirm || !row_enabled(menu, self.command) {
                    return None;
                }
                match self.command {
                    ROW_ATTACK => self.aim(fight, unit, menu.attack.clone(), MenuLevel::Commands),
                    row @ (1 | 2) => match &menu.commands[row - 1].kind {
                        CommandKind::Direct(entry) => {
                            self.aim(fight, unit, entry.clone(), MenuLevel::Commands)
                        }
                        CommandKind::List(_) => {
                            self.level = MenuLevel::List(ListKind::Ability(row - 1));
                            self.list = self.remembered_list(unit, ListKind::Ability(row - 1));
                        }
                        CommandKind::Unavailable => {}
                    },
                    ROW_ITEM => {
                        self.level = MenuLevel::List(ListKind::Items);
                        self.list = self.remembered_list(unit, ListKind::Items);
                    }
                    ROW_DEFEND => self.aim(fight, unit, menu.defend.clone(), MenuLevel::Commands),
                    ROW_CHANGE => self.aim(fight, unit, menu.change.clone(), MenuLevel::Commands),
                    _ => {}
                }
                None
            }
            MenuLevel::List(kind) => {
                let list = list_of(menu, kind);
                self.list = step(self.list, list.len(), up, down);
                if cancel {
                    self.level = MenuLevel::Commands;
                } else if confirm
                    && let Some(entry) = list.get(self.list)
                    && usable(fight, unit, entry)
                {
                    self.aim(fight, unit, entry.clone(), MenuLevel::List(kind));
                }
                None
            }
            MenuLevel::Target { entry, back, all } => {
                let candidates = targets(fight, unit, &entry);
                self.target = step(self.target, candidates.len(), up || left, down || right);
                if cancel {
                    self.level = *back;
                    return None;
                }
                if pressed & pad::R1 != 0 && entry.can_toggle_all() {
                    self.level = MenuLevel::Target {
                        entry,
                        back,
                        all: !all,
                    };
                    return None;
                }
                if !confirm {
                    return None;
                }
                let chosen = if all {
                    candidates
                } else {
                    vec![candidates.get(self.target).copied()?]
                };
                let targets = chosen
                    .into_iter()
                    .map(|target| fight.entity(target))
                    .collect::<Option<Vec<_>>>()?;
                let command = BattleCommand {
                    actor: fight.entity(unit)?,
                    action: entry.action.clone(),
                    targets,
                };
                self.memorize(unit, &back);
                self.reset_to(None);
                Some(command)
            }
        }
    }

    /// The menu for `unit`, back on its first rows, or on its remembered
    /// ones with [`Self::remember`].
    fn reset_to(&mut self, unit: Option<usize>) {
        self.unit = unit;
        self.level = MenuLevel::Commands;
        self.command = 0;
        self.list = 0;
        self.target = 0;
        if self.remember
            && let Some(unit) = unit
        {
            self.command = self.memo_command.get(&unit).copied().unwrap_or(0);
        }
    }

    fn remembered_list(&self, unit: usize, kind: ListKind) -> usize {
        if self.remember {
            self.memo_list.get(&(unit, kind)).copied().unwrap_or(0)
        } else {
            0
        }
    }

    /// Records the rows a chosen command came from
    /// (`BattleHUD` `_commandCursorMemorize`).
    fn memorize(&mut self, unit: usize, back: &MenuLevel) {
        self.memo_command.insert(unit, self.command);
        if let MenuLevel::List(kind) = back {
            self.memo_list.insert((unit, *kind), self.list);
        }
    }

    /// `BattleHUD.IsNativeEnableAtb` with Config `ATB: Wait`
    /// (`cfg.atb == 1`): the gauges run only while no member is choosing or
    /// the menu is on the command window.
    pub fn wait_stalls_atb(&self) -> bool {
        self.unit.is_some() && self.level != MenuLevel::Commands
    }

    /// Opens the target cursor for `entry`, on the caster for allies and
    /// on the first enemy otherwise (`BattleHUD.SetTargetDefault`); an
    /// `All*` entry covers the side from the start.
    fn aim(&mut self, fight: &LiveFight, unit: usize, entry: MenuEntry, back: MenuLevel) {
        let candidates = targets(fight, unit, &entry);
        self.target = candidates
            .iter()
            .position(|&candidate| candidate == unit)
            .unwrap_or(0);
        let all = entry.always_all();
        self.level = MenuLevel::Target {
            entry,
            back: Box::new(back),
            all,
        };
    }

    /// The units under the target cursor.
    pub fn targeted(&self, fight: &LiveFight) -> Vec<usize> {
        let (MenuLevel::Target { entry, all, .. }, Some(unit)) = (&self.level, self.unit) else {
            return Vec::new();
        };
        let candidates = targets(fight, unit, entry);
        if *all {
            candidates
        } else {
            candidates.get(self.target).copied().into_iter().collect()
        }
    }
}
