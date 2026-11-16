//! Command slots and the command queue (milestone B5).
//!
//! Ports the linked-list behavior of `btl_cmd.cs` over an index arena. A
//! `CMD_DATA` is a slot id; the queue is an ordered `Vec` of slot ids (the
//! source's `cmd_queue.next` chain, `cmd_queue` itself is the head node).
//! Each unit owns six slots (`btl_cmd.cs:39-49`):
//! 0 normal commands, 1 counters, 2 death or petrify, 3 first half of a double
//! cast or a Phantom, 4 trance, 5 reraise. One extra slot is `cmd_escape`.

// The id comparisons mirror the C# (`< X || > Y`) one for one.
#![allow(clippy::manual_range_contains)]

use super::command::BattleCommand;

/// `BattleCommandId` values (`Memoria/Data/Battle/BattleCommandId.cs`).
pub mod cmd {
    pub const NONE: u32 = 0;
    pub const ATTACK: u32 = 1;
    pub const MAGIC_SWORD: u32 = 31;
    pub const ITEM: u32 = 14;
    pub const THROW: u32 = 15;
    pub const COUNTER: u32 = 49;
    pub const MAGIC_COUNTER: u32 = 50;
    pub const AUTO_POTION: u32 = 51;
    pub const JUMP: u32 = 3;
    pub const DEFEND: u32 = 4;
    pub const JUMP_IN_TRANCE: u32 = 12;
    pub const SPEAR: u32 = 10;
    pub const DOUBLE_WHITE_MAGIC: u32 = 21;
    pub const DOUBLE_BLACK_MAGIC: u32 = 23;
    pub const ENEMY_ATK: u32 = 47;
    pub const BOUNDARY_CHECK: u32 = 48;
    pub const RUSH_ATTACK: u32 = 52;
    pub const ENEMY_COUNTER: u32 = 53;
    pub const ENEMY_DYING: u32 = 54;
    pub const ENEMY_REACTION: u32 = 55;
    pub const SYS_ESCAPE: u32 = 56;
    pub const SYS_PHANTOM: u32 = 57;
    pub const SYS_LAST_PHOENIX: u32 = 58;
    pub const SYS_TRANS: u32 = 59;
    pub const SYS_DEAD: u32 = 60;
    pub const SYS_RERAISE: u32 = 61;
    pub const SYS_STONE: u32 = 62;
    pub const SCRIPT_COUNTER_1: u32 = 63;
    pub const SCRIPT_COUNTER_2: u32 = 64;
    pub const SCRIPT_COUNTER_3: u32 = 65;
    pub const BOUNDARY_UPPER_CHECK: u32 = 99;
}

/// `btl_util.IsCommandDeclarable` (`FF9/btl_util.cs:343`).
pub fn is_declarable(cmd_no: u32) -> bool {
    cmd_no < cmd::BOUNDARY_CHECK || cmd_no > cmd::BOUNDARY_UPPER_CHECK
}

/// Slots per unit (`btl_cmd.cs:144`).
pub const SLOTS_PER_UNIT: usize = 6;

/// `command_mode_index` (`FF9/command_mode_index.cs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmdMode {
    Inspection,
    SelectVfx,
    Loop,
    Idle,
    Done,
}

/// Slot id (index into [`CommandQueue::slots`]).
pub type CmdId = usize;

/// What the data tables say about a command: the `BattleCommand` the B4
/// formulas read plus the `AA_DATA.Info` flags `btl_cmd.cs` reads. Built by a
/// catalog (`BattleHooks::spec`); the engine has no table loader of its own.
///
/// Field mapping from `tantalus_assets::pc::battle_tables::ActionRow`:
/// `command.script_id` = `btl_ref.prog_no`, `command.target_type` =
/// `info.cursor`, `default_ally` = `info.def_cur`, `for_dead` = `info.dead`
/// (`CMD_DATA.SetAAData`, `Global/CMD_DATA.cs:38-54`).
#[derive(Debug, Clone, PartialEq)]
pub struct CommandSpec {
    pub command: BattleCommand,
    pub default_ally: bool,
    pub for_dead: bool,
}

/// One `CMD_DATA`.
#[derive(Debug, Clone, PartialEq)]
pub struct CmdSlot {
    /// `regist`: index of the owner unit.
    pub regist: Option<usize>,
    /// Which of the owner's six slots (`SLOTS_PER_UNIT` for `cmd_escape`).
    pub slot: usize,
    pub cmd_no: u32,
    pub sub_no: i32,
    pub tar_id: u16,
    /// `info.priority`.
    pub priority: bool,
    /// `info.cursor` (bit 0: multi-target).
    pub cursor: u8,
    pub mode: CmdMode,
    /// `IsShortRange`.
    pub short_range: bool,
    pub spec: Option<CommandSpec>,
    /// Ticks spent in `CMD_MODE_LOOP` (injected-timing counter).
    pub loop_ticks: u32,
    /// The effect of this command has been applied (injected-timing flag).
    pub effect_done: bool,
    /// `info.reflec`: 0 none, 1 reflected onto `reflec_targets`, 2 partly
    /// reflected (the non-reflecting targets resolve first).
    pub reflec_mode: u8,
    /// `reflec.tar_id[0..4]`.
    pub reflec_targets: [u16; 4],
    /// `info.HasCheckedReflect`.
    pub has_checked_reflect: bool,
    /// `info.cover == 1`: a cover replaced a target of this command.
    pub covered: bool,
    /// `CMD_DATA.magic_caster_id`: the `btl_id` of the Magic Sword supporter
    /// (`btl_cmd.cs:264-266`); 0 for every other command. Reset by
    /// `ClearCommand` (`btl_cmd.cs:59`).
    pub magic_caster_id: u16,
}

impl CmdSlot {
    /// `ClearCommand` (`btl_cmd.cs:54`) plus `info.Reset()`
    /// (`Global/CMD_DATA.cs:90-102`): mode back to inspection.
    pub fn cleared(regist: Option<usize>, slot: usize) -> Self {
        Self {
            regist,
            slot,
            cmd_no: cmd::NONE,
            sub_no: 0,
            tar_id: 0,
            priority: false,
            cursor: 0,
            mode: CmdMode::Inspection,
            short_range: false,
            spec: None,
            loop_ticks: 0,
            effect_done: false,
            reflec_mode: 0,
            reflec_targets: [0; 4],
            has_checked_reflect: false,
            covered: false,
            magic_caster_id: 0,
        }
    }
}

/// Command arena plus the ordered queue.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandQueue {
    pub slots: Vec<CmdSlot>,
    /// `cmd_queue.next` chain, front first.
    pub queue: Vec<CmdId>,
    /// `cmd_status` (`FF9StateBattleSystem`): bit 0 = an escape is pending.
    pub cmd_status: u16,
}

impl CommandQueue {
    /// Slots for `unit_count` units, then `cmd_escape` registered to
    /// `escape_owner` (`btl_init.cs:278`: the first player).
    pub fn new(unit_count: usize, escape_owner: Option<usize>) -> Self {
        let mut slots = Vec::with_capacity(unit_count * SLOTS_PER_UNIT + 1);
        for u in 0..unit_count {
            for k in 0..SLOTS_PER_UNIT {
                slots.push(CmdSlot::cleared(Some(u), k));
            }
        }
        slots.push(CmdSlot::cleared(escape_owner, SLOTS_PER_UNIT));
        Self {
            slots,
            queue: Vec::new(),
            // `InitCommandSystem` (`btl_cmd.cs:74`): `cmd_status = 2`.
            cmd_status: 2,
        }
    }

    /// Slot id of unit `u`'s slot `k`.
    pub fn slot_of(&self, u: usize, k: usize) -> CmdId {
        u * SLOTS_PER_UNIT + k
    }

    /// Slot id of `cmd_escape`.
    pub fn escape_slot(&self) -> CmdId {
        self.slots.len() - 1
    }

    /// `CheckCommandQueued` (`btl_cmd.cs:823`).
    pub fn is_queued(&self, id: CmdId) -> bool {
        self.queue.contains(&id)
    }

    /// `CheckUsingCommand` (`btl_cmd.cs:812`): queued or in `running`.
    pub fn is_using(&self, id: CmdId, running: &[CmdId]) -> bool {
        self.queue.contains(&id) || running.contains(&id)
    }

    /// `CheckSpecificCommand` (`btl_cmd.cs:790`).
    pub fn has_specific(&self, unit: usize, cmd_no: u32, running: &[CmdId]) -> bool {
        self.queue
            .iter()
            .chain(running.iter())
            .any(|&c| self.slots[c].regist == Some(unit) && self.slots[c].cmd_no == cmd_no)
    }

    /// `CheckSpecificCommand2` (`btl_cmd.cs:801`).
    pub fn has_specific_any(&self, cmd_no: u32, running: &[CmdId]) -> bool {
        self.queue
            .iter()
            .chain(running.iter())
            .any(|&c| self.slots[c].cmd_no == cmd_no)
    }

    /// `EnqueueCommand` + `InsertCommand` (`btl_cmd.cs:831-874`). The loop
    /// walks `cp` from the head node; position `i` below means `cp` is the
    /// element before index `i` (the head for `i == 0`) and `cp.next` is
    /// `queue[i]`. `InsertCommand` links `cmd` after `cp` and sets
    /// `info.stat = 1` (not stored here).
    pub fn enqueue(&mut self, id: CmdId) {
        let (cmd_no, priority) = {
            let c = &self.slots[id];
            (c.cmd_no, c.priority)
        };
        let mut i = 0usize;
        loop {
            let next = self.queue.get(i).copied();
            if priority && let Some(n) = next {
                let nn = self.slots[n].cmd_no;
                let n_priority = self.slots[n].priority;
                if cmd_no > cmd::SYS_TRANS || cmd_no == cmd::ENEMY_DYING {
                    if (nn < cmd::SYS_DEAD || nn > cmd::BOUNDARY_UPPER_CHECK)
                        && nn != cmd::ENEMY_DYING
                    {
                        self.insert_at(id, i);
                        return;
                    }
                } else if cmd_no == cmd::SYS_TRANS {
                    if (nn < cmd::SYS_TRANS || nn > cmd::BOUNDARY_UPPER_CHECK)
                        && nn != cmd::ENEMY_DYING
                    {
                        self.insert_at(id, i);
                        return;
                    }
                } else if !n_priority || is_declarable(nn) {
                    self.insert_at(id, i);
                    return;
                }
            } else if next.is_none() {
                self.insert_at(id, i);
                return;
            }
            i += 1;
        }
    }

    fn insert_at(&mut self, id: CmdId, pos: usize) {
        // `InsertCommand`: `if (cmd == cp) return;` cannot happen here
        // because `cmd` is not in the chain when enqueued.
        self.queue.insert(pos, id);
    }

    /// `DequeueCommand` (`btl_cmd.cs:780`) for the element at `pos`:
    /// `info.stat = 0; info.priority = 0`; with `escape_check` a removed
    /// `SysEscape` clears `cmd_status` bit 0.
    fn dequeue_at(&mut self, pos: usize, escape_check: bool) -> CmdId {
        let id = self.queue.remove(pos);
        self.slots[id].priority = false;
        if escape_check && self.slots[id].cmd_no == cmd::SYS_ESCAPE {
            self.cmd_status &= 0xFFFE;
        }
        id
    }

    /// `KillCommand` (`btl_cmd.cs:671`): remove one queued command (no escape
    /// check). Returns true when it was queued.
    pub fn kill_command(&mut self, id: CmdId) -> bool {
        if let Some(pos) = self.queue.iter().position(|&c| c == id) {
            self.dequeue_at(pos, false);
            true
        } else {
            false
        }
    }

    /// `KillStandardCommands` (`btl_cmd.cs:704`): remove every queued command
    /// of `unit` whose id is below `EnemyDying` or above the upper boundary.
    /// Returns true when the main command (slot 0) was among them.
    pub fn kill_standard_commands(&mut self, unit: usize) -> bool {
        let mut cancel_main = false;
        let mut i = 0;
        while i < self.queue.len() {
            let id = self.queue[i];
            let c = &self.slots[id];
            if c.regist == Some(unit)
                && (c.cmd_no < cmd::ENEMY_DYING || c.cmd_no > cmd::BOUNDARY_UPPER_CHECK)
            {
                if c.slot == 0 {
                    cancel_main = true;
                }
                self.dequeue_at(i, true);
            } else {
                i += 1;
            }
        }
        cancel_main
    }

    /// `KillSpecificCommand` (`btl_cmd.cs:745`). Slot 0 goes through
    /// `KillMainCommand` (`:686`), which does not run the escape check; the
    /// others do.
    pub fn kill_specific(&mut self, unit: usize, cmd_no: u32) -> bool {
        let Some(pos) = self
            .queue
            .iter()
            .position(|&c| self.slots[c].regist == Some(unit) && self.slots[c].cmd_no == cmd_no)
        else {
            return false;
        };
        let main = self.slots[self.queue[pos]].slot == 0;
        self.dequeue_at(pos, !main);
        true
    }

    /// `KillAllCommand` (`btl_cmd.cs:1155`): remove every queued command whose
    /// id is below `SysTrans` or above the upper boundary
    /// (`ManageDequeueCommand`, `:1177`, runs the escape check), then
    /// `cmd_status &= 0xFFF3`. Returns the removed ids so the caller can reset
    /// `sel_mode` for declarable ones whose owner is alive.
    pub fn kill_all(&mut self) -> Vec<CmdId> {
        let mut removed = Vec::new();
        let mut i = 0;
        while i < self.queue.len() {
            let n = self.slots[self.queue[i]].cmd_no;
            if n < cmd::SYS_TRANS || n > cmd::BOUNDARY_UPPER_CHECK {
                removed.push(self.dequeue_at(i, true));
            } else {
                i += 1;
            }
        }
        self.cmd_status &= 0xFFF3;
        removed
    }

    /// `KillNormalCommand` (`btl_cmd.cs:1142`): remove every queued command
    /// except `EnemyDying` and those in `SysLastPhoenix..=BoundaryUpperCheck`.
    pub fn kill_normal(&mut self) -> Vec<CmdId> {
        let mut removed = Vec::new();
        let mut i = 0;
        while i < self.queue.len() {
            let n = self.slots[self.queue[i]].cmd_no;
            if n != cmd::ENEMY_DYING && (n < cmd::SYS_LAST_PHOENIX || n > cmd::BOUNDARY_UPPER_CHECK)
            {
                removed.push(self.dequeue_at(i, true));
            } else {
                i += 1;
            }
        }
        removed
    }

    /// `ClearCommand` + `ClearReflecData` on a slot.
    pub fn clear(&mut self, id: CmdId) {
        let (regist, slot) = (self.slots[id].regist, self.slots[id].slot);
        self.slots[id] = CmdSlot::cleared(regist, slot);
    }

    /// First queued command ready to run:
    /// `GetFirstCommandReadyToDequeue` (`btl_cmd.cs:440-486`) at battle speed
    /// mode 0 (the `Speed >= 4` and busy-caster branches do not apply).
    /// `cannot_act(unit)`, `dead(unit)` and `is_enemy(unit)` read unit
    /// status; `any_player_alive` is the `hasPlayerAlive` scan.
    pub fn first_ready(
        &self,
        any_player_alive: bool,
        cannot_act: &dyn Fn(usize) -> bool,
        dead: &dyn Fn(usize) -> bool,
        is_enemy: &dyn Fn(usize) -> bool,
    ) -> Option<CmdId> {
        for &id in &self.queue {
            let c = &self.slots[id];
            if let Some(r) = c.regist {
                let exempt = matches!(
                    c.cmd_no,
                    cmd::SYS_DEAD
                        | cmd::SYS_RERAISE
                        | cmd::SYS_STONE
                        | cmd::SYS_ESCAPE
                        | cmd::SYS_LAST_PHOENIX
                );
                if (cannot_act(r) && !exempt) || (dead(r) && c.cmd_no == cmd::SYS_PHANTOM) {
                    continue;
                }
                if !any_player_alive
                    && is_enemy(r)
                    && (c.cmd_no == cmd::ENEMY_ATK || c.cmd_no == cmd::ENEMY_COUNTER)
                {
                    continue;
                }
            }
            return Some(id);
        }
        None
    }
}
