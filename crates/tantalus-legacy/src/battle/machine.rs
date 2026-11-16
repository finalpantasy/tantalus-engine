//! Deterministic, I/O-free battle state machine (milestone B5).
//!
//! One call of [`Battle::tick`] is one `HonoluluBattleMain.UpdateBattleFrame`
//! (`Global/Honolulu/HonoluluBattleMain.cs:669-696`): `battle.BattleMain()`
//! for the current phase, then the ATB add (`YMenu_ManagerActiveTime`). The
//! shipped rate is 15 ticks per second ([`super::atb::BATTLE_TPS`]).
//!
//! Scope and assumptions (all from the pinned Memoria `c716e1fa`):
//!
//! * Battle speed mode `Configuration.Battle.Speed = 0` (the shipped default,
//!   `Memoria.ini`). Modes 1 to 5 change dequeue and ATB rules and are not
//!   ported.
//! * `btl_scene.Info.WinPose = false` (no victory camera), no
//!   `NoGameOver` rules, `IsDebug = false`, no cheats.
//! * Animation, VFX and sequencer timing is not ported. Where the source waits
//!   for it the wait is an injected tick count ([`Timing`]); the real values
//!   come from model animations and the SFX sequencer and are unknown here.
//!   Defaults are 0 (no wait).
//! * Enemy AI is the event engine and is not ported: [`BattleHooks::enemy_atb`]
//!   supplies the action an enemy takes when its gauge fills.
//! * Reflect, Cover and Protect Girls, the Counter and Auto-Potion support
//!   abilities, Item, Auto-Potion and Throw commands, Magic Sword and Double
//!   Cast live in [`super::reactions`]. Steal, jump, summon decisions, other
//!   support abilities, Auto-Life, Phantom recast and Eiko's Rebirth Flame are
//!   not ported.

#![allow(clippy::needless_range_loop)]

use super::atb::{self, GaugeInput, GaugeStep, StartType};
use super::calc::{CalcEnv, CalcError, calc_main};
use super::command::{BattleCommand, ItemData};
use super::cost;
use super::flee::{self, EscapeSides};
use super::queue::{CmdId, CmdMode, CommandQueue, CommandSpec, SLOTS_PER_UNIT, cmd, is_declarable};
use super::rng::BattleRandom;
use super::status::{self as st, StatusCtx, StatusEnv, StatusTable, alter_status, remove_status};
use super::targeting::{self, TargetCheck, TargetView, target_type};
use super::unit::BattleUnit;

/// `BattleAbilityId.Attack` (`Actions.csv` row `Attack;176`), used by the
/// automatic Attack of Berserk and Confuse (`BerserkStatusScript.cs:34`).
pub const ABILITY_ATTACK: i32 = 176;

/// `FF9StateGlobal.BTL_RESULT_*` (`Global/ff9/State/FF9StateGlobal.cs:10-17`).
pub mod result {
    pub const VICTORY: u8 = 1;
    pub const VICTORY_NO_POSE: u8 = 2;
    pub const DEFEAT: u8 = 3;
    pub const ESCAPE: u8 = 4;
    pub const ENEMY_FLEE: u8 = 7;
}

/// `FF9StateBattleSystem.btl_phase` values the port reaches
/// (`FF9StateBattleSystem.cs:10-18`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// `PHASE_EVENT`: the battle script owns the scene (special starts and
    /// after `SetBattleData` 32), with the ATB stopped.
    Event,
    /// `PHASE_MENU_ON`: one idle pass before the ATB starts (`SetBattleData`
    /// 35).
    MenuOn,
    /// `PHASE_NORMAL`.
    Normal,
    /// `PHASE_MENU_OFF`.
    MenuOff,
    /// `PHASE_VICTORY`.
    Victory,
    /// `PHASE_DEFEAT`.
    Defeat,
    /// `PHASE_CLOSE`.
    Close,
}

/// `btl_seq` while in `PHASE_MENU_OFF` (`FF9StateBattleSystem.cs:22-25`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuOffSeq {
    Victory,
    Defeat,
    /// `SEQ_MENU_OFF_EVENT`: winding down into [`Phase::Event`].
    Event,
    Escape,
}

/// `btl_seq` in the closing phases (`SEQ_VICTORY_FADEOUT` / `SEQ_VICTORY_WAIT`,
/// `SEQ_DEFEATCLOSE_FADEOUT` / `SEQ_DEFEATCLOSE_WAIT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndSeq {
    FadeOut,
    Wait,
}

/// Wait times the source takes from animation or the sequencer. Injected
/// because those data are not ported; every field defaults to 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Timing {
    /// Ticks an enemy stays in `die_seq` 2 (`btl_mot.cs:306-314`: waits for the
    /// dying animation to end).
    pub enemy_death_anim_ticks: u32,
    /// Ticks from a player gaining Death to `die_seq` 6
    /// (`btl_mot.cs:360-365`: the down motion must end).
    pub player_death_ticks: u32,
    /// Ticks in `CMD_MODE_LOOP` before the effect is applied (the SFX frame
    /// that calls `ExecVfxCommand`, `btl_cmd.cs:1526`).
    pub effect_after_loop_ticks: u32,
    /// Ticks in `CMD_MODE_LOOP` before the command finishes (`SFX.isRunning`
    /// and the sequencer, `btl_cmd.cs:1441-1443`). Must be at least
    /// `effect_after_loop_ticks`.
    pub loop_total_ticks: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BattleConfig {
    /// `Configuration.Battle.ATBSpeedFactor` as configured (clamped at use).
    pub atb_speed_factor: f32,
    /// `FF9StateSystem.Settings.cfg.btl_speed` (0, 1 or 2).
    pub btl_speed_config: u8,
    /// `Configuration.Battle.IsKeepRestTimeInBattle`.
    pub keep_rest_time: bool,
    /// `btl_scene.Info.Runaway`.
    pub runaway: bool,
    pub timing: Timing,
}

impl Default for BattleConfig {
    fn default() -> Self {
        Self {
            atb_speed_factor: atb::DEFAULT_ATB_SPEED_FACTOR,
            btl_speed_config: atb::DEFAULT_BTL_SPEED_CONFIG,
            keep_rest_time: false,
            runaway: true,
            timing: Timing::default(),
        }
    }
}

/// Per-unit battle scheduling data that `BattleUnit` does not carry.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitSched {
    /// `max.at`.
    pub max_atb: i16,
    /// `cur.at_coef` (`SByte`).
    pub atb_coef: i8,
    /// `bi.atb != 0`.
    pub atb_active: bool,
    /// `BTL_DATA.sel_mode`.
    pub sel_mode: u8,
    /// The unit is in `btl_list` (false after `DelCharacter`).
    pub in_list: bool,
    /// `bi.target != 0`.
    pub targetable: bool,
    pub out_of_reach: bool,
    /// `bi.disappear != 0` (after escaping).
    pub disappeared: bool,
    /// `die_seq` (`BTL_DATA`).
    pub die_seq: u8,
    /// Enemy `info.die_fade_rate`, 32 at init (`btl_init.cs:45`).
    pub die_fade_rate: u8,
    /// Injected wait counter, see [`Timing`].
    pub die_wait: u32,
    /// `player[charId].mpCostFactor` (default 100, `ff9play.cs:313`).
    pub mp_cost_factor: i32,
    /// Experience this enemy adds to the battle bonus when it finishes dying
    /// (`btl_sys.SetBonus`, `btl_sys.cs:215`).
    pub bonus_exp: u32,
}

impl UnitSched {
    pub fn new(unit: &BattleUnit, base_coef: i32) -> Self {
        Self {
            max_atb: atb::max_atb(unit.dexterity),
            atb_coef: atb::setup_atb_coef(base_coef),
            atb_active: true,
            sel_mode: 0,
            in_list: true,
            targetable: true,
            out_of_reach: false,
            disappeared: false,
            die_seq: 0,
            die_fade_rate: 32,
            die_wait: 0,
            mp_cost_factor: 100,
            bonus_exp: 0,
        }
    }
}

/// What an enemy does when its gauge fills (`SetEnemyCommand`
/// arguments, `btl_cmd.cs:346`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnemyAction {
    pub cmd_no: u32,
    pub sub_no: i32,
    pub tar_id: u16,
}

/// Data and decisions the engine cannot own.
pub trait BattleHooks {
    /// `btl_util.GetCommandAction` + `GetCommandScriptId` for the command
    /// (`btl_cmd.cs:169-170`): the effect data, or `None` for commands that
    /// have no calculator effect (system commands).
    fn spec(&mut self, cmd_no: u32, sub_no: i32, caster: &BattleUnit) -> Option<CommandSpec>;

    /// The enemy AI request on a full gauge
    /// (`RequestAction(EnemyAtk, ...)`, `HonoluluBattleMain.cs:529`). `None`
    /// means the request did not run an action.
    fn enemy_atb(&mut self, _battle: &Battle, _unit: usize) -> Option<EnemyAction> {
        None
    }

    /// `ff9item.GetItemEffect(item)` for an Item command. `None` means the
    /// item has no use effect and the command is refused.
    fn item(&mut self, _item_id: i32) -> Option<ItemData> {
        None
    }

    /// `Weapon.Find(item).Power` for a Throw command (`ThrowScript`,
    /// `Command.Weapon.Power`). `None` means the item has no weapon row.
    fn item_weapon_power(&mut self, _item_id: i32) -> Option<i32> {
        None
    }

    /// A `RequestAction` the effect pipeline raises after a hit on an enemy
    /// (`SBattleCalculator.CalcResult`, `SBattleCalculator.cs:315-325`): the
    /// AI function for counter, dying or reaction may start. The default
    /// ignores it.
    fn enemy_reaction(&mut self, _battle: &Battle, _request: EnemyReactionRequest) {}

    /// Counter and dying commands the enemy AI queued since the last call
    /// (`BTLCMD` at script level 1 and 0, `SetEnemyCommand`): `(caster
    /// btl_id, action)`. Polled once per tick, before the ATB pass.
    fn enemy_reaction_commands(&mut self, _battle: &Battle) -> Vec<(u16, EnemyAction)> {
        Vec::new()
    }

    /// The battle scripts' frame (`EventEngine.ServiceEvents`), once per
    /// tick before `BattleMain`. Returns the `SetBattleData` calls the
    /// machine applies, as `(code, value)`; see [`Battle::battle_code`].
    fn frame(&mut self, _battle: &Battle) -> Vec<(u32, i32)> {
        Vec::new()
    }

    /// Unit members the scripts wrote during [`BattleHooks::frame`]
    /// (`SetCharacterData`), applied before `BattleMain` so the machine's own
    /// changes that tick (damage, gauges) are not overwritten by a stale
    /// view.
    fn unit_writes(
        &mut self,
    ) -> std::collections::BTreeMap<tantalus_contracts::EntityId, super::host::UnitView> {
        std::collections::BTreeMap::new()
    }
}

/// Which `RequestAction` flavor a hit raises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnemyReactionKind {
    /// `RequestAction(EnemyCounter, ...)`: script level 1, tag 6.
    Counter,
    /// `RequestAction(EnemyDying, ...)`: script level 0, tag 9.
    Dying,
    /// `RequestAction(EnemyReaction, ...)`: script level 2, tag 7.
    Reaction,
}

/// `RequestAction(cmd, target, reactCaster, reactCmdId, reactSubId)`
/// (`EventEngine.cs:285`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnemyReactionRequest {
    pub kind: EnemyReactionKind,
    /// `target`: the `btl_id` of the enemy that was hit.
    pub target_id: u16,
    /// `reactCaster`: the `btl_id` of the unit that hit it.
    pub caster_id: u16,
    /// `reactCmdId`: the command that hit it.
    pub cmd_no: u32,
    /// `reactSubId`.
    pub sub_no: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BattleEvent {
    /// A unit's gauge reached max (before the `PreventATBConfirm` check).
    AtbFull {
        unit: usize,
    },
    /// A player is waiting for input (`AddPlayerToReady`).
    PlayerReady {
        unit: usize,
    },
    CommandQueued {
        unit: usize,
        cmd_no: u32,
        tar_id: u16,
    },
    CommandStarted {
        unit: usize,
        cmd_no: u32,
    },
    /// A condition in `CMD_MODE_INSPECTION` failed; the command is dropped.
    CommandRejected {
        unit: usize,
        cmd_no: u32,
    },
    EffectApplied {
        caster: usize,
        target: usize,
        target_hp: u32,
    },
    CommandFinished {
        unit: usize,
        cmd_no: u32,
    },
    /// A tick of Poison, Venom, Regen, Doom or Gradual Petrify.
    StatusTick {
        unit: usize,
        status: u8,
    },
    StatusExpired {
        unit: usize,
        status: u8,
    },
    UnitDied {
        unit: usize,
    },
    UnitRemoved {
        unit: usize,
    },
    /// `CheckReflec` redirected a command: `mode` 1 aims it at `targets`,
    /// mode 2 keeps the non-reflecting targets and replays on `targets`.
    Reflected {
        caster: usize,
        reflecting: u16,
        targets: [u16; 4],
        mode: u8,
    },
    /// A cover replaced `target_id` with the covering unit `cover_id`.
    Covered {
        target_id: u16,
        cover_id: u16,
    },
    /// `DecideMagicSword` refused a Magic Sword spell
    /// (`BattleMesages.CombinationFailed`, `btl_cmd.cs:1631-1641`).
    CombinationFailed {
        unit: usize,
    },
    /// Steal ran on `target`: `item` went into the inventory, or `None`
    /// ("Couldn't steal anything").
    Stole {
        caster: usize,
        target: usize,
        item: Option<u32>,
    },
    /// Jump: `unit` left the field.
    Jumped {
        unit: usize,
    },
    /// Spear landed: `unit` is back.
    Landed {
        unit: usize,
    },
    /// Eat or Cook ran on `target`.
    Ate {
        caster: usize,
        target: usize,
        result: super::calc::EatResult,
    },
    /// `ItemUse`: one copy of `item` was removed from the inventory.
    ItemUsed {
        unit: usize,
        item: i32,
        remaining: u32,
    },
    TranceEntered {
        unit: usize,
    },
    TranceEnded {
        unit: usize,
    },
    PhaseChanged {
        phase: Phase,
    },
    Result {
        result: u8,
    },
    BattleOver,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BattleError {
    Calc(CalcError),
    /// A source expression divides by zero (the C# throws).
    DivideByZero(&'static str),
    /// A command needs table data the hooks did not provide.
    UnknownCommand {
        cmd_no: u32,
        sub_no: i32,
    },
    /// A source path this port leaves out was reached.
    Unported(&'static str),
}

impl std::fmt::Display for BattleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Calc(e) => write!(f, "calculator: {e}"),
            Self::DivideByZero(w) => write!(f, "division by zero in {w}"),
            Self::UnknownCommand { cmd_no, sub_no } => {
                write!(f, "no spec for command {cmd_no} sub {sub_no}")
            }
            Self::Unported(w) => write!(f, "not ported: {w}"),
        }
    }
}

impl std::error::Error for BattleError {}

impl From<CalcError> for BattleError {
    fn from(e: CalcError) -> Self {
        Self::Calc(e)
    }
}

/// A unit in the air after Jump ([`Battle::jumps`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jump {
    pub countdown: i32,
    pub target: u16,
    /// Spear is queued.
    pub landing: bool,
}

pub struct Battle {
    pub units: Vec<BattleUnit>,
    /// Units in the air after Jump (`JumpStatusScript`): the countdown to
    /// the Spear (`SpearCountdown`), its target and whether it is queued.
    pub jumps: std::collections::BTreeMap<usize, Jump>,
    pub sched: Vec<UnitSched>,
    pub cmds: CommandQueue,
    /// `cur_cmd_list`.
    pub running: Vec<CmdId>,
    /// `UIManager.Battle.ReadyQueue`: players waiting for input, in order.
    pub ready_players: Vec<usize>,
    pub phase: Phase,
    pub menu_off_seq: MenuOffSeq,
    pub end_seq: EndSeq,
    /// `ff9.btl_result` once decided.
    pub result: Option<u8>,
    /// `ManageBattleEnd` returned (the battle is over).
    pub over: bool,
    /// `_commandEnable` (`FF9BMenu_IsEnable`).
    pub menu_enabled: bool,
    /// Wait-mode pause: `IsNativeEnableAtb` is false while a player has a
    /// command menu open (`BattleHUD.Public.cs:795-805`). Supplied by the
    /// caller; false means the gauge runs.
    pub atb_paused: bool,
    pub config: BattleConfig,
    pub table: StatusTable,
    /// Completed ticks (`btl_cnt` also counts them, `battle.cs:209`).
    pub tick_no: u32,
    /// `btl_fade_time`.
    pub fade_time: u32,
    /// `btl_escape_fade`, 32 at init (`btl_sys.cs:32`).
    pub escape_fade: i32,
    /// `btl_escape_key != 0`.
    pub escape_key: bool,
    /// `BTL_FLAG_ABILITY_FLEE` (set by the Flee ability variant).
    pub ability_flee: bool,
    /// `party.escape_no`.
    pub escape_count: u32,
    /// `btl_bonus.exp`.
    pub bonus_exp: u32,
    /// Item inventory the Item command reads and decrements
    /// (`ff9item.FF9Item_GetCount` / `FF9Item_Remove`): item id to count. An
    /// id that is absent has count 0. The host owns the real inventory and
    /// seeds this from it.
    pub inventory: std::collections::BTreeMap<i32, u32>,
    /// Magic Sword links in effect: the host builds one per
    /// `BattleMagicSwordSet` whose supporter is in the battle
    /// (`BattleHUD.SetAbilityMagic`, `BattleHUD.cs:1473`).
    pub magic_swords: Vec<super::reactions::MagicSwordSet>,
    /// `(tick, event)` in order; the tick is 1-based.
    pub log: Vec<(u32, BattleEvent)>,
}

fn ids_of(mask: u64) -> impl Iterator<Item = u8> {
    (0u8..64).filter(move |i| mask & (1u64 << i) != 0)
}

impl Battle {
    /// A battle in `PHASE_NORMAL`. `units` are in `btl_list` order (players
    /// first, then enemies). Gauge values in `units[i].cur_atb` are kept; use
    /// [`atb::initial_player_atb`] / [`atb::initial_enemy_atb`] to set them.
    pub fn new(units: Vec<BattleUnit>, config: BattleConfig, table: StatusTable) -> Self {
        let base = atb::base_atb_coef(config.btl_speed_config);
        let sched: Vec<UnitSched> = units.iter().map(|u| UnitSched::new(u, base)).collect();
        let escape_owner = units.iter().position(|u| u.is_player);
        let cmds = CommandQueue::new(units.len(), escape_owner);
        Self {
            units,
            sched,
            cmds,
            running: Vec::new(),
            ready_players: Vec::new(),
            phase: Phase::Normal,
            menu_off_seq: MenuOffSeq::Victory,
            end_seq: EndSeq::FadeOut,
            result: None,
            over: false,
            menu_enabled: true,
            atb_paused: false,
            config,
            table,
            tick_no: 0,
            fade_time: 0,
            escape_fade: 32,
            escape_key: false,
            ability_flee: false,
            escape_count: 0,
            bonus_exp: 0,
            inventory: std::collections::BTreeMap::new(),
            jumps: std::collections::BTreeMap::new(),
            magic_swords: Vec::new(),
            log: Vec::new(),
        }
    }

    pub(super) fn emit(&mut self, e: BattleEvent) {
        self.log.push((self.tick_no + 1, e));
    }

    /// Applies `cfg.btl_speed` after construction (the Config screen's Battle
    /// Speed): each unit's gauge coefficient moves from the old base to the new
    /// one, keeping Haste and Slow.
    pub fn set_btl_speed(&mut self, speed: u8) {
        let old = self.base_coef();
        self.config.btl_speed_config = speed;
        let new = self.base_coef();
        for sched in &mut self.sched {
            sched.atb_coef = if sched.atb_coef == atb::hasted_coef(old) {
                atb::hasted_coef(new)
            } else if sched.atb_coef == atb::slowed_coef(old) {
                atb::slowed_coef(new)
            } else {
                atb::setup_atb_coef(new)
            };
        }
    }

    fn base_coef(&self) -> i32 {
        atb::base_atb_coef(self.config.btl_speed_config)
    }

    fn status_env(&self) -> StatusEnv {
        StatusEnv {
            phase_after_enter: true,
            battle_speed: 0,
            menu_off_phase: self.phase == Phase::MenuOff,
        }
    }

    /// `FF9BMenu_IsEnableAtb` at battle speed 0 (`BattleHUD.Public.cs:776-793`).
    pub fn atb_runs(&self) -> bool {
        self.menu_enabled && !self.atb_paused
    }

    fn dead(&self, i: usize) -> bool {
        self.units[i].is_under_any_status(st::DEATH)
    }

    fn views(&self) -> Vec<TargetView> {
        (0..self.units.len())
            .filter(|&i| self.sched[i].in_list)
            .map(|i| TargetView {
                btl_id: self.units[i].id,
                is_player: self.units[i].is_player,
                dead: self.dead(i),
                targetable: self.sched[i].targetable,
                out_of_reach: self.sched[i].out_of_reach,
            })
            .collect()
    }

    /// `btl_util.IsBtlBusy(btl, ANY_CURRENT)` (`btl_util.cs:176`): the unit is
    /// the caster or a target of a running command.
    fn busy_current(&self, i: usize) -> bool {
        let id = self.units[i].id;
        self.running.iter().any(|&c| {
            let s = &self.cmds.slots[c];
            s.regist == Some(i) || s.tar_id & id != 0
        })
    }

    fn queued_caster(&self, i: usize) -> bool {
        self.cmds
            .queue
            .iter()
            .any(|&c| self.cmds.slots[c].regist == Some(i))
    }

    fn any_player_alive(&self) -> bool {
        (0..self.units.len()).any(|i| {
            self.sched[i].in_list
                && self.units[i].is_player
                && !self.units[i].is_under_any_status(st::BATTLE_END | st::DEATH)
        })
    }

    // -- status bookkeeping ------------------------------------------------

    fn snapshot(&self) -> Vec<u64> {
        self.units.iter().map(|u| u.status_current).collect()
    }

    /// Run the side effects the B4 status port leaves out (see
    /// `status.rs::apply_script`), for every status that changed since
    /// `prev`. Removals are handled before additions. Sources are the
    /// `Apply` / `Remove` bodies in `Memoria/Scripts/DefaultStatus/` and the
    /// tail of `btl_stat.AlterStatus` (`btl_stat.cs:73-99`).
    fn sync(&mut self, prev: &[u64]) {
        for i in 0..self.units.len() {
            let removed = prev[i] & !self.units[i].status_current;
            if removed != 0 {
                self.on_removed(i, removed);
            }
        }
        for i in 0..self.units.len() {
            let added = self.units[i].status_current & !prev[i];
            if added != 0 {
                self.on_added(i, added);
            }
            // `PetrifyStatusScript.Apply` (`PetrifyStatusScript.cs:13-18`)
            // queues `SysStone` when B4 deferred the petrify.
            let stone = self.cmds.slot_of(i, 2);
            if self.units[i].petrify_command_pending
                && !self.cmds.is_using(stone, &self.running)
                && self.cmds.slots[stone].cmd_no != cmd::SYS_STONE
            {
                self.queue_system_command(stone, cmd::SYS_STONE, self.units[i].id);
            }
        }
    }

    /// `SetCommand` for a system command a status script queues
    /// (`btl_cmd.cs:153-311`): no-op when the slot is already queued or
    /// running, otherwise fill the slot and enqueue it.
    fn queue_system_command(&mut self, id: CmdId, cmd_no: u32, tar_id: u16) {
        if self.cmds.is_using(id, &self.running) {
            return;
        }
        self.raw_set_command(id, cmd_no, 0, tar_id, 0, None);
        self.cmds.enqueue(id);
    }

    /// Apply `alter_status` to a unit and run the side effects
    /// ([`Battle::sync`]). For callers and tests that inflict a status
    /// outside a command.
    pub fn alter_status_on(
        &mut self,
        unit: usize,
        id: u8,
        rng: &mut dyn BattleRandom,
    ) -> st::AlterResult {
        let prev = self.snapshot();
        let env = self.status_env();
        let ctx = StatusCtx {
            table: &self.table,
            env: &env,
        };
        let r = alter_status(&mut self.units[unit], id, false, &ctx, rng);
        self.sync(&prev);
        r
    }

    /// `remove_status` plus the side effects, see [`Battle::alter_status_on`].
    pub fn remove_status_on(&mut self, unit: usize, id: u8) -> u32 {
        let prev = self.snapshot();
        let env = self.status_env();
        let r = remove_status(&mut self.units[unit], id, &env);
        self.sync(&prev);
        r
    }

    fn on_removed(&mut self, i: usize, bits: u64) {
        let base = self.base_coef();
        if bits & (st::HASTE | st::SLOW) != 0 {
            // `HasteStatusScript.Remove` / `SlowStatusScript.Remove`.
            self.sched[i].atb_coef = atb::setup_atb_coef(base);
        }
        if bits & st::TRANCE != 0 {
            // `TranceStatusScript.Remove` (`TranceStatusScript.cs:48-62`).
            self.units[i].trance = 0;
            let slot = self.cmds.slot_of(i, 4);
            self.queue_system_command(slot, cmd::SYS_TRANS, self.units[i].id);
            self.emit(BattleEvent::TranceEnded { unit: i });
        }
        if bits & (st::PETRIFY | st::GRADUAL_PETRIFY) != 0 {
            // `PetrifyStatusScript.Remove`, `GradualPetrifyStatusScript.Remove`.
            self.cmds.kill_specific(i, cmd::SYS_STONE);
        }
        if bits & (st::BERSERK | st::CONFUSE) != 0 {
            // `BerserkStatusScript.Remove`, `ConfuseStatusScript.Remove`.
            self.status_command_cancel(i);
        }
        if bits & st::DEATH != 0 {
            // `DeathStatusScript.Remove` (`DeathStatusScript.cs:46-62`).
            self.sched[i].die_seq = 0;
            self.sched[i].die_wait = 0;
            let using = self
                .running
                .iter()
                .find(|&&c| self.cmds.slots[c].regist == Some(i))
                .copied();
            if using.is_none_or(|c| !is_declarable(self.cmds.slots[c].cmd_no)) {
                self.sched[i].sel_mode = 0;
            }
        }
    }

    fn on_added(&mut self, i: usize, bits: u64) {
        let base = self.base_coef();
        if bits & st::HASTE != 0 {
            self.sched[i].atb_coef = atb::hasted_coef(base);
        }
        if bits & st::SLOW != 0 {
            self.sched[i].atb_coef = atb::slowed_coef(base);
        }
        if bits & st::CMD_CANCEL != 0 {
            // `btl_stat.cs:77-78`.
            self.status_command_cancel(i);
        }
        if bits & st::TRANCE != 0 {
            // `TranceStatusScript.Apply` (`TranceStatusScript.cs:44-51`).
            let slot = self.cmds.slot_of(i, 4);
            self.queue_system_command(slot, cmd::SYS_TRANS, self.units[i].id);
            self.emit(BattleEvent::TranceEntered { unit: i });
        }
        if bits & st::DEATH != 0 {
            self.on_death(i);
        }
        if self.units[i].is_under_any_status(st::STOP_ATB) {
            // `btl_stat.cs:98-99`.
            self.sched[i].atb_active = false;
        }
        if bits & st::BATTLE_END != 0 {
            // `btl_stat.cs:95-96`.
            self.check_battle_phase(i);
        }
    }

    /// `DeathStatusScript.Apply` (`DeathStatusScript.cs:15-44`). B4 already
    /// killed the unit and zeroed the gauge.
    fn on_death(&mut self, i: usize) {
        self.emit(BattleEvent::UnitDied { unit: i });
        if self.units[i].is_player {
            // Injected wait for the down motion (`btl_mot.cs:360-365`).
            self.sched[i].die_seq = 1;
            self.sched[i].die_wait = self.config.timing.player_death_ticks;
        } else {
            if self.sched[i].die_seq == 0 {
                // `!AttackOnDeath || !busy`: `AttackOnDeath` is enemy data and
                // is assumed false here.
                self.sched[i].die_seq = 1;
            }
            self.check_forecast_menu_off(i);
        }
        if self.units[i].is_under_any_status(st::TRANCE)
            && self.cmds.kill_specific(i, cmd::SYS_TRANS)
        {
            let prev = self.snapshot();
            let env = self.status_env();
            remove_status(&mut self.units[i], 14, &env);
            self.sync_removal_only(&prev);
            self.units[i].trance = self.units[i].trance.min(254);
        }
    }

    fn sync_removal_only(&mut self, prev: &[u64]) {
        for i in 0..self.units.len() {
            let removed = prev[i] & !self.units[i].status_current;
            if removed != 0 {
                self.on_removed(i, removed);
            }
        }
    }

    /// `btl_stat.StatusCommandCancel` (`btl_stat.cs:27-37`).
    fn status_command_cancel(&mut self, i: usize) {
        if self.units[i].is_player {
            self.ready_players.retain(|&u| u != i);
        }
        if !self.cmds.kill_standard_commands(i) {
            return;
        }
        self.sched[i].atb_active = false;
        if self.units[i].is_player {
            self.units[i].cur_atb = 0;
        }
        self.sched[i].sel_mode = 0;
    }

    /// `btl_sys.CheckForecastMenuOff` (`btl_sys.cs:140-150`).
    fn check_forecast_menu_off(&mut self, dying: usize) {
        for j in 0..self.units.len() {
            if self.sched[j].in_list && !self.units[j].is_player && !self.dead(j) && j != dying {
                return;
            }
        }
        self.menu_enabled = false;
        let removed = self.cmds.kill_normal();
        self.after_dequeue(&removed);
        self.escape_key = false;
    }

    /// `ManageDequeueCommand` tail (`btl_cmd.cs:1177-1197`): a declarable
    /// command removed from a living owner resets its `sel_mode`.
    fn after_dequeue(&mut self, removed: &[CmdId]) {
        for &c in removed {
            let (no, regist) = (self.cmds.slots[c].cmd_no, self.cmds.slots[c].regist);
            if let Some(r) = regist
                && is_declarable(no)
                && !self.dead(r)
            {
                self.sched[r].sel_mode = 0;
            }
        }
    }

    /// `btl_sys.CheckBattlePhase` (`btl_sys.cs:52-122`), default
    /// (no overload) path; the Eiko Rebirth Flame branch (`:96-114`) is not
    /// ported.
    fn check_battle_phase(&mut self, dying: usize) {
        let side = self.units[dying].is_player;
        for j in 0..self.units.len() {
            if !self.sched[j].in_list || self.units[j].is_player != side {
                continue;
            }
            let end_full = self.units[j].is_under_any_status(st::BATTLE_END | st::DEATH);
            if !end_full || (self.dead(j) && self.sched[j].die_seq != 6) {
                return;
            }
        }
        if !side {
            if !self.any_player_alive() {
                return;
            }
            if self.cmds.has_specific_any(cmd::ENEMY_DYING, &self.running) {
                return;
            }
            if self.menu_off_seq != MenuOffSeq::Defeat || self.phase != Phase::MenuOff {
                self.menu_off_seq = MenuOffSeq::Victory;
            }
        } else {
            self.menu_off_seq = MenuOffSeq::Defeat;
        }
        self.menu_enabled = false;
        self.phase = Phase::MenuOff;
        self.emit(BattleEvent::PhaseChanged {
            phase: Phase::MenuOff,
        });
        let removed = self.cmds.kill_all();
        self.after_dequeue(&removed);
    }

    // -- point data and death sequence ----------------------------------------

    /// `btl_para.CheckPointData` (`btl_para.cs:77-92`).
    fn check_point_data(&mut self, i: usize, rng: &mut dyn BattleRandom) {
        let prev = self.snapshot();
        let env = self.status_env();
        {
            let u = &mut self.units[i];
            if u.cur_hp > u.max_hp {
                u.cur_hp = u.max_hp;
            }
            if u.cur_mp > u.max_mp {
                u.cur_mp = u.max_mp;
            }
        }
        let ctx = StatusCtx {
            table: &self.table,
            env: &env,
        };
        let s = st::check_point_status(&mut self.units[i], &ctx, rng);
        if s & st::DEATH != 0 && !self.units[i].is_under_any_status(st::DEATH) {
            alter_status(&mut self.units[i], 8, false, &ctx, rng);
        }
        self.sync(&prev);
    }

    /// `btl_mot.DieSequence` (`btl_mot.cs:286-370`). The enemy path follows
    /// the source steps with the animation wait injected at step 2; the
    /// player path collapses the motion steps into `die_wait` and then
    /// `DecidePlayerDieSequence` (`:432-463`, Auto-Life cancel not ported).
    fn die_sequence(&mut self, i: usize) {
        if self.sched[i].die_seq == 0 {
            return;
        }
        if self.units[i].is_player {
            if self.sched[i].die_seq >= 6 {
                return;
            }
            if self.sched[i].die_wait > 0 {
                self.sched[i].die_wait -= 1;
                return;
            }
            self.sched[i].die_seq = 6;
            self.check_battle_phase(i);
            return;
        }
        match self.sched[i].die_seq {
            1 => {
                if self.queued_caster(i) {
                    return;
                }
                self.sched[i].die_seq = 2;
                self.sched[i].die_wait = self.config.timing.enemy_death_anim_ticks;
            }
            2 => {
                if self.sched[i].die_wait > 0 {
                    self.sched[i].die_wait -= 1;
                    return;
                }
                self.sched[i].die_seq = 3;
            }
            3 => {
                self.sched[i].die_seq = 4;
                if self.sched[i].die_fade_rate == 0 {
                    self.sched[i].die_seq = 5;
                } else {
                    self.sched[i].die_fade_rate -= 2;
                }
            }
            4 => {
                if self.sched[i].die_fade_rate == 0 {
                    self.sched[i].die_seq = 5;
                } else {
                    self.sched[i].die_fade_rate -= 2;
                }
            }
            5 => {
                if self.busy_current(i) {
                    return;
                }
                self.bonus_exp += self.sched[i].bonus_exp;
                self.sched[i].die_seq = 6;
                let escaping =
                    self.phase == Phase::MenuOff && self.menu_off_seq == MenuOffSeq::Escape;
                if !escaping {
                    self.check_battle_phase(i);
                }
                self.sched[i].in_list = false;
                self.emit(BattleEvent::UnitRemoved { unit: i });
            }
            _ => {}
        }
    }

    // -- commands ---------------------------------------------------------------

    /// The body of `btl_cmd.SetCommand` that follows the spec lookup
    /// (`btl_cmd.cs:268-310`) without the Double Cast branch. `spec` is the
    /// resolved `GetCommandAction`.
    fn raw_set_command(
        &mut self,
        id: CmdId,
        cmd_no: u32,
        sub_no: i32,
        tar_id: u16,
        cursor: u8,
        spec: Option<CommandSpec>,
    ) {
        let slot = &mut self.cmds.slots[id];
        slot.cmd_no = cmd_no;
        slot.sub_no = sub_no;
        slot.tar_id = tar_id;
        slot.cursor = cursor;
        slot.short_range = spec.as_ref().is_some_and(|s| s.command.is_short_range);
        slot.spec = spec;
        slot.mode = CmdMode::Inspection;
        slot.loop_ticks = 0;
        slot.effect_done = false;
        slot.reflec_mode = 0;
        slot.reflec_targets = [0; 4];
        slot.has_checked_reflect = false;
        slot.covered = false;
        slot.magic_caster_id = 0;
        slot.priority = !is_declarable(cmd_no);
    }

    /// `btl_cmd.SetCommand` (`btl_cmd.cs:153-311`). Returns false when the
    /// slot is already in use or an escape is already pending.
    /// Arguments mirror `btl_cmd.SetCommand`'s signature.
    #[allow(clippy::too_many_arguments)]
    pub fn set_command(
        &mut self,
        id: CmdId,
        cmd_no: u32,
        sub_no: i32,
        tar_id: u16,
        cursor: u8,
        force_priority: bool,
        hooks: &mut dyn BattleHooks,
    ) -> Result<bool, BattleError> {
        self.set_command_inner(
            id,
            cmd_no,
            sub_no,
            tar_id,
            cursor,
            force_priority,
            None,
            hooks,
            None,
        )
    }

    /// [`Battle::set_command`] with a random source, needed by the Double Cast
    /// branch that picks a random target (`Comn.randomID`, `btl_cmd.cs:215`).
    #[allow(clippy::too_many_arguments)]
    pub fn set_command_with_rng(
        &mut self,
        id: CmdId,
        cmd_no: u32,
        sub_no: i32,
        tar_id: u16,
        cursor: u8,
        force_priority: bool,
        hooks: &mut dyn BattleHooks,
        rng: &mut dyn BattleRandom,
    ) -> Result<bool, BattleError> {
        self.set_command_inner(
            id,
            cmd_no,
            sub_no,
            tar_id,
            cursor,
            force_priority,
            None,
            hooks,
            Some(rng),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn set_command_inner(
        &mut self,
        id: CmdId,
        cmd_no: u32,
        sub_no: i32,
        tar_id: u16,
        cursor: u8,
        force_priority: bool,
        custom_mp_cost: Option<i32>,
        hooks: &mut dyn BattleHooks,
        rng: Option<&mut dyn BattleRandom>,
    ) -> Result<bool, BattleError> {
        if self.cmds.is_using(id, &self.running) {
            return Ok(false);
        }
        self.cmds.slots[id].cmd_no = cmd_no;
        self.cmds.slots[id].sub_no = sub_no;
        if cmd_no == cmd::SYS_ESCAPE {
            if self.cmds.cmd_status & 1 != 0 {
                return Ok(false);
            }
            self.cmds.cmd_status |= 1;
        }
        let regist = self.cmds.slots[id].regist;
        let mut spec = match regist {
            Some(r) => hooks.spec(cmd_no, sub_no, &self.units[r]),
            None => None,
        };
        let is_player = regist.is_some_and(|r| self.units[r].is_player);
        if matches!(cmd_no, cmd::ITEM | cmd::AUTO_POTION) && is_player {
            spec = Some(self.item_spec(spec, cmd_no, sub_no, hooks)?);
        } else if cmd_no == cmd::THROW && is_player {
            spec = spec.map(|s| self.throw_spec(s, sub_no, hooks));
        }
        if let (Some(s), Some(r)) = (&spec, regist)
            && s.command.script_id == 80
            && is_player
        {
            let double = s.clone();
            return self.queue_double_cast(id, r, &double, cmd_no, tar_id, hooks, rng);
        }
        if let (Some(s), Some(mp)) = (spec.as_mut(), custom_mp_cost) {
            s.command.custom_mp_cost = Some(mp);
        }
        self.raw_set_command(id, cmd_no, sub_no, tar_id, cursor, spec);
        if let Some(r) = regist
            && let Some(set) = self.magic_sword_of(r, cmd_no, sub_no)
        {
            // `btl_cmd.cs:262-266`; a missing supporter unit gives 0.
            self.cmds.slots[id].magic_caster_id = self.units[set.supporter].id;
        }
        if force_priority {
            self.cmds.slots[id].priority = true;
        }
        if let Some(r) = regist
            && self.cmds.slots[id].slot == 0
        {
            // `RemoveStatuses(RemoveOnMainCommand)` = Defend (`:305-309`).
            let prev = self.snapshot();
            let env = self.status_env();
            remove_status(&mut self.units[r], 15, &env);
            self.sync(&prev);
        }
        self.cmds.enqueue(id);
        if let Some(r) = regist {
            self.emit(BattleEvent::CommandQueued {
                unit: r,
                cmd_no,
                tar_id,
            });
        }
        Ok(true)
    }

    /// A player's input for unit `unit` (slot 0). Marks `sel_mode` as the HUD
    /// does (`BattleHUD.cs:1696`) and removes the player from the ready list.
    pub fn player_command(
        &mut self,
        unit: usize,
        cmd_no: u32,
        sub_no: i32,
        tar_id: u16,
        cursor: u8,
        hooks: &mut dyn BattleHooks,
    ) -> Result<bool, BattleError> {
        let id = self.cmds.slot_of(unit, 0);
        self.sched[unit].sel_mode = 1;
        let ok = self.set_command(id, cmd_no, sub_no, tar_id, cursor, false, hooks)?;
        self.ready_players.retain(|&u| u != unit);
        Ok(ok)
    }

    /// `btl_cmd.SetEnemyCommand` (`btl_cmd.cs:346-438`) for `EnemyAtk`
    /// (slot 0), `EnemyCounter` and `EnemyDying` (slot 1) and
    /// `ScriptCounter1..3` (slots 3 to 5, queued as `EnemyCounter`).
    ///
    /// Not modeled: `EnemyDying` while slot 1 holds another command. The
    /// source then adds a seventh command (`cmd[6]`, `btl_cmd.cs:401-410`);
    /// the six-slot arena has none, so that request is dropped.
    pub fn set_enemy_command(
        &mut self,
        i: usize,
        action: EnemyAction,
        hooks: &mut dyn BattleHooks,
    ) -> Result<(), BattleError> {
        if self.cmds.cmd_status & 1 != 0 || self.units[i].is_under_any_status(st::PREVENT_ENEMY_CMD)
        {
            self.sched[i].sel_mode = 0;
            return Ok(());
        }
        // `IsSpecialHPInDyingState` (`btl_para.cs:350`): a non-dying boss
        // under 10000 HP and alone on its side stops attacking.
        if action.cmd_no != cmd::ENEMY_DYING
            && self.phase == Phase::Normal
            && self.units[i].non_dying_boss
            && self.units[i].cur_hp < 10000
            && self.enemy_side_mask() == self.units[i].id
        {
            self.sched[i].sel_mode = 0;
            return Ok(());
        }
        let (slot_k, cmd_no) = match action.cmd_no {
            cmd::ENEMY_ATK => {
                if self.phase != Phase::Normal {
                    self.sched[i].sel_mode = 0;
                    return Ok(());
                }
                (0, cmd::ENEMY_ATK)
            }
            cmd::ENEMY_COUNTER => (1, cmd::ENEMY_COUNTER),
            cmd::ENEMY_DYING => (1, cmd::ENEMY_DYING),
            cmd::SCRIPT_COUNTER_1 => (3, cmd::ENEMY_COUNTER),
            cmd::SCRIPT_COUNTER_2 => (4, cmd::ENEMY_COUNTER),
            cmd::SCRIPT_COUNTER_3 => (5, cmd::ENEMY_COUNTER),
            _ => {
                self.sched[i].sel_mode = 0;
                return Ok(());
            }
        };
        let id = self.cmds.slot_of(i, slot_k);
        if self.cmds.is_using(id, &self.running) {
            if cmd_no == cmd::ENEMY_ATK {
                self.sched[i].sel_mode = 0;
            }
            return Ok(());
        }
        self.cmds.clear(id);
        let spec = hooks.spec(cmd_no, action.sub_no, &self.units[i]);
        let cursor = match &spec {
            Some(s) => {
                let t = s.command.target_type;
                if t <= target_type::MANY_ENEMY || t >= target_type::SELF {
                    0
                } else {
                    1
                }
            }
            None => 0,
        };
        self.raw_set_command(id, cmd_no, action.sub_no, action.tar_id, cursor, spec);
        if slot_k == 0 {
            let prev = self.snapshot();
            let env = self.status_env();
            remove_status(&mut self.units[i], 15, &env);
            self.sync(&prev);
        }
        self.cmds.enqueue(id);
        self.emit(BattleEvent::CommandQueued {
            unit: i,
            cmd_no,
            tar_id: action.tar_id,
        });
        Ok(())
    }

    /// `btl_scrp.GetBattleID(1)`: every listed enemy's `btl_id` bit.
    fn enemy_side_mask(&self) -> u16 {
        (0..self.units.len())
            .filter(|&j| self.sched[j].in_list && !self.units[j].is_player)
            .fold(0, |m, j| m | self.units[j].id)
    }

    /// Queue the counter and dying commands the AI raised (`SetEnemyCommand`
    /// from `BTLCMD`, `DoEventCode.cs:1177-1215`).
    fn poll_enemy_reactions(&mut self, hooks: &mut dyn BattleHooks) -> Result<(), BattleError> {
        for (caster_id, action) in hooks.enemy_reaction_commands(self) {
            let Some(i) = (0..self.units.len())
                .find(|&j| self.sched[j].in_list && self.units[j].id == caster_id)
            else {
                continue;
            };
            self.set_enemy_command(i, action, hooks)?;
        }
        Ok(())
    }

    // -- flee ---------------------------------------------------------------------

    /// `BattleCalculator.CanEscape` (`BattleCalculator.cs:305`).
    pub fn can_escape(&self) -> bool {
        let any = (0..self.units.len()).any(|i| {
            self.sched[i].in_list
                && self.units[i].is_player
                && !self.units[i].is_under_any_status(st::CANNOT_ESCAPE)
        });
        flee::can_escape(self.config.runaway, any)
    }

    /// The party flee roll once the key has been held long enough:
    /// `btl_sys.CheckEscape(true)` (`btl_sys.cs:306-321`) -> script 56 ->
    /// `TryEscape`. Returns true when `SysEscape` was queued.
    pub fn flee_attempt(
        &mut self,
        hooks: &mut dyn BattleHooks,
        rng: &mut dyn BattleRandom,
    ) -> Result<bool, BattleError> {
        if self.cmds.cmd_status & 1 != 0 || !self.config.runaway || !self.atb_runs() {
            return Ok(false);
        }
        if !self.can_escape() {
            return Ok(false);
        }
        // `TryEscape` returns when the current command belongs to an enemy
        // (`BattleCalculator.cs:351-353`).
        let cur_enemy = self.running.iter().any(|&c| {
            self.cmds.slots[c]
                .regist
                .is_some_and(|r| !self.units[r].is_player)
        });
        if cur_enemy {
            return Ok(false);
        }
        let mut sides = EscapeSides {
            player_count: 0,
            enemy_count: 0,
            player_levels: 0,
            enemy_levels: 0,
        };
        for i in 0..self.units.len() {
            if !self.sched[i].in_list {
                continue;
            }
            if self.units[i].is_player {
                sides.player_count += 1;
                sides.player_levels += i32::from(self.units[i].level);
            } else {
                sides.enemy_count += 1;
                sides.enemy_levels += i32::from(self.units[i].level);
            }
        }
        let rate = flee::escape_rate(sides).map_err(BattleError::DivideByZero)?;
        if flee::try_escape_roll(rate, rng) {
            let id = self.cmds.escape_slot();
            return self.set_command(id, cmd::SYS_ESCAPE, 1, 15, 1, false, hooks);
        }
        Ok(false)
    }

    /// The Flee ability, script 57 (`0057_FleeScript.cs`): when
    /// `CanEscape()` it queues `SysEscape` with `sub_no 0` and the party
    /// mask, no roll.
    pub fn flee_ability(&mut self, hooks: &mut dyn BattleHooks) -> Result<bool, BattleError> {
        if self.cmds.cmd_status & 1 != 0 {
            return Ok(false);
        }
        if self.can_escape() {
            let id = self.cmds.escape_slot();
            return self.set_command(id, cmd::SYS_ESCAPE, 0, 15, 1, false, hooks);
        }
        Ok(false)
    }

    // -- the tick -----------------------------------------------------------------

    /// One `UpdateBattleFrame`.
    pub fn tick(
        &mut self,
        hooks: &mut dyn BattleHooks,
        rng: &mut dyn BattleRandom,
    ) -> Result<(), BattleError> {
        if self.over {
            return Ok(());
        }
        for (code, value) in hooks.frame(self) {
            self.battle_code(code, value);
        }
        let writes = hooks.unit_writes();
        if !writes.is_empty() {
            super::host::apply_unit_views(self, &writes);
        }
        self.battle_main(hooks, rng)?;
        if self.menu_enabled {
            self.poll_enemy_reactions(hooks)?;
            self.process_active_time(hooks, rng)?;
        }
        self.tick_no += 1;
        Ok(())
    }

    /// `battle.BattleMain` (`battle.cs:83-211`).
    fn battle_main(
        &mut self,
        hooks: &mut dyn BattleHooks,
        rng: &mut dyn BattleRandom,
    ) -> Result<(), BattleError> {
        match self.phase {
            Phase::Event => self.idle_loop(),
            Phase::MenuOn => {
                // `BattleIdleLoop` returns true once every unit is in its
                // idle motion; there are no motions here, so at once.
                self.idle_loop();
                self.phase = Phase::Normal;
                self.menu_enabled = true;
                self.emit(BattleEvent::PhaseChanged {
                    phase: Phase::Normal,
                });
            }
            Phase::Normal => self.battle_main_loop(hooks, rng)?,
            Phase::MenuOff => {
                self.menu_enabled = false;
                self.trailing_loop(hooks, rng)?;
                if self.menu_off_seq != MenuOffSeq::Escape || self.phase != Phase::MenuOff {
                    self.escape_key = false;
                }
            }
            Phase::Victory | Phase::Defeat | Phase::Close => {
                self.idle_loop();
                self.closing_phase();
            }
        }
        // `battle.cs:181-208`: every enemy gone without a victory sequence.
        if self.phase == Phase::Normal
            && self.running.is_empty()
            && !(0..self.units.len()).any(|i| self.sched[i].in_list && !self.units[i].is_player)
            && (0..self.units.len()).any(|i| {
                self.sched[i].in_list
                    && !self.units[i].is_under_any_status(st::BATTLE_END | st::DEATH)
            })
        {
            self.menu_enabled = false;
            self.escape_key = false;
            self.cmds.cmd_status &= 0xFFFD;
            let removed = self.cmds.kill_all();
            self.after_dequeue(&removed);
            let mut r = if self.bonus_exp > 0 {
                result::VICTORY
            } else {
                result::ENEMY_FLEE
            };
            if r == result::VICTORY {
                r = result::VICTORY_NO_POSE;
            }
            self.result = Some(r);
            self.phase = Phase::Close;
            self.end_seq = EndSeq::FadeOut;
            self.emit(BattleEvent::Result { result: r });
            self.emit(BattleEvent::PhaseChanged {
                phase: Phase::Close,
            });
        }
        Ok(())
    }

    /// Starts in [`Phase::Event`] with the ATB stopped, for scenes with
    /// `Info.SpecialStart` (`SFX.cs:1612-1615`): the battle script runs the
    /// opening and calls `SetBattleData` 35 to start the ATB.
    pub fn begin_special_start(&mut self) {
        self.phase = Phase::Event;
        self.menu_enabled = false;
    }

    /// `btl_scrp.SetBattleData` codes 32-35 (`btl_scrp.cs:774-825`); other
    /// codes are script-side state and are ignored here.
    ///
    /// * 32 Disable ATB: menu off with the event sequence.
    /// * 33 End battle (`PHASE_EVENT` only): `btl_result = value`. With no
    ///   scene win pose, Victory becomes `VICTORY_NO_POSE` and the battle
    ///   closes; the win-pose path (`SEQ_MENU_OFF_VICTORY_WITH_ENEMY`) is
    ///   not ported.
    /// * 34 Game Over (`PHASE_EVENT` only): the defeat phase.
    /// * 35 Enable ATB (`PHASE_EVENT` only): `PHASE_MENU_ON`.
    pub fn battle_code(&mut self, code: u32, value: i32) {
        match code {
            32 => {
                self.menu_enabled = false;
                self.escape_key = false;
                self.cmds.cmd_status &= 0xFFFD;
                self.phase = Phase::MenuOff;
                self.menu_off_seq = MenuOffSeq::Event;
                let removed = self.cmds.kill_all();
                self.after_dequeue(&removed);
                self.emit(BattleEvent::PhaseChanged {
                    phase: Phase::MenuOff,
                });
            }
            33 if self.phase == Phase::Event => {
                let mut r = value as u8;
                if r == result::VICTORY {
                    r = result::VICTORY_NO_POSE;
                }
                self.result = Some(r);
                self.phase = Phase::Close;
                self.end_seq = EndSeq::FadeOut;
                self.emit(BattleEvent::Result { result: r });
                self.emit(BattleEvent::PhaseChanged {
                    phase: Phase::Close,
                });
            }
            34 if self.phase == Phase::Event => {
                self.result = Some(result::DEFEAT);
                self.phase = Phase::Defeat;
                self.end_seq = EndSeq::FadeOut;
                self.emit(BattleEvent::Result {
                    result: result::DEFEAT,
                });
                self.emit(BattleEvent::PhaseChanged {
                    phase: Phase::Defeat,
                });
            }
            35 if self.phase == Phase::Event => {
                self.phase = Phase::MenuOn;
                self.emit(BattleEvent::PhaseChanged {
                    phase: Phase::MenuOn,
                });
            }
            _ => {}
        }
    }

    /// `battle.BattleMainLoop` (`battle.cs:213-243`).
    fn battle_main_loop(
        &mut self,
        hooks: &mut dyn BattleHooks,
        rng: &mut dyn BattleRandom,
    ) -> Result<(), BattleError> {
        for i in 0..self.units.len() {
            if !self.sched[i].in_list {
                continue;
            }
            self.check_point_data(i, rng);
            self.check_status_loop(i, rng);
            self.jump_countdown(i, hooks)?;
        }
        self.command_engine(hooks, rng)
    }

    /// `battle.BattleIdleLoop` reduced to the death sequence
    /// (`battle.cs:245-281`; the rest is animation).
    fn idle_loop(&mut self) {
        for i in 0..self.units.len() {
            if self.sched[i].in_list {
                self.die_sequence(i);
            }
        }
    }

    /// The `PHASE_VICTORY` / `PHASE_DEFEAT` / `PHASE_CLOSE` cases of
    /// `BattleMain` (`battle.cs:140-177`), `WinPose = false`. After the result
    /// is decided one tick turns `FadeOut` into `Wait`, then
    /// `btl_fade_time++ > 32` ends the battle 34 ticks later. The defeat song
    /// fade (`btl_util.ManageBattleSong`) is not ported, so a defeat does not
    /// reach `over`.
    fn closing_phase(&mut self) {
        if self.phase == Phase::Defeat {
            return;
        }
        match self.end_seq {
            EndSeq::FadeOut => self.end_seq = EndSeq::Wait,
            EndSeq::Wait => {
                let t = self.fade_time;
                self.fade_time += 1;
                if t > 32 {
                    self.over = true;
                    self.emit(BattleEvent::BattleOver);
                }
            }
        }
    }

    /// `battle.BattleTrailingLoop` (`battle.cs:283-486`) for the Victory,
    /// Defeat and Escape sequences.
    fn trailing_loop(
        &mut self,
        hooks: &mut dyn BattleHooks,
        rng: &mut dyn BattleRandom,
    ) -> Result<(), BattleError> {
        let mut proceed = self.cmds.queue.is_empty() && self.running.is_empty();
        for i in 0..self.units.len() {
            if !self.sched[i].in_list {
                continue;
            }
            if !self.sched[i].disappeared {
                match self.menu_off_seq {
                    MenuOffSeq::Victory => {
                        self.check_point_data(i, rng);
                        let u = &self.units[i];
                        let enemy_blocks = !u.is_player && !u.is_under_any_status(st::BATTLE_END);
                        let player_blocks =
                            u.is_player && self.dead(i) && self.sched[i].die_seq != 6;
                        if enemy_blocks || player_blocks {
                            proceed = false;
                        }
                    }
                    MenuOffSeq::Defeat => {
                        let u = &self.units[i];
                        if u.is_player
                            && self.sched[i].die_seq != 6
                            && !u.is_under_any_status(st::BATTLE_END)
                        {
                            proceed = false;
                        }
                    }
                    MenuOffSeq::Event => {
                        // `battle.cs:307-320`. The SysTrans/SysDead/
                        // SysReraise/SysStone queue exclusions and the idle
                        // motion check are not ported.
                        if !self.cmds.queue.is_empty() && self.running.is_empty() {
                            proceed = true;
                        }
                        self.check_point_data(i, rng);
                        if !self.running.is_empty() {
                            proceed = false;
                        }
                        if self.dead(i) && self.sched[i].die_seq != 6 {
                            proceed = false;
                        }
                    }
                    MenuOffSeq::Escape => {
                        let removed = self.cmds.kill_all();
                        self.after_dequeue(&removed);
                        if self.dead(i) {
                            if self.sched[i].die_seq != 6 {
                                proceed = false;
                            }
                        } else if self.units[i].is_player
                            && !self.units[i].is_under_any_status(st::CANNOT_ESCAPE)
                        {
                            if self.escape_fade <= 0 {
                                self.sched[i].disappeared = true;
                            } else {
                                proceed = false;
                                if self.escape_fade == 32 {
                                    self.escape_fade -= 2;
                                }
                            }
                        }
                    }
                }
            }
            self.die_sequence(i);
        }
        if self.menu_off_seq == MenuOffSeq::Escape && self.escape_fade < 32 && self.escape_fade != 0
        {
            self.escape_fade -= 2;
        }
        if !proceed {
            return self.command_engine(hooks, rng);
        }
        match self.menu_off_seq {
            MenuOffSeq::Victory => {
                self.result = Some(result::VICTORY);
                self.phase = Phase::Victory;
            }
            MenuOffSeq::Defeat => {
                self.result = Some(result::DEFEAT);
                self.phase = Phase::Defeat;
            }
            MenuOffSeq::Event => {
                // `battle.cs:426-441`: the script takes over; no result.
                self.phase = Phase::Event;
                let removed = self.cmds.kill_all();
                self.after_dequeue(&removed);
                self.emit(BattleEvent::PhaseChanged {
                    phase: Phase::Event,
                });
                return Ok(());
            }
            MenuOffSeq::Escape => {
                self.result = Some(result::ESCAPE);
                self.phase = Phase::Close;
            }
        }
        self.end_seq = EndSeq::FadeOut;
        if let Some(r) = self.result {
            self.emit(BattleEvent::Result { result: r });
        }
        let phase = self.phase;
        self.emit(BattleEvent::PhaseChanged { phase });
        Ok(())
    }

    // -- status loop ----------------------------------------------------------------

    /// `btl_stat.CheckStatusLoop` (`btl_stat.cs:269-314`).
    fn check_status_loop(&mut self, i: usize, rng: &mut dyn BattleRandom) {
        if self.dead(i) {
            self.die_sequence(i);
            return;
        }
        if !self.units[i].is_under_any_status(st::STOP_ATB) {
            self.sched[i].atb_active = true;
        }
        if !self.atb_runs() || !self.sched[i].atb_active {
            return;
        }
        let coef = i32::from(self.sched[i].atb_coef);
        let mut remove: u64 = 0;
        let prev = self.snapshot();
        for id in ids_of(self.units[i].status_current & st::OPR_COUNT) {
            let opr = self.units[i].opr.get(&id).copied().unwrap_or(0);
            if opr <= 0 {
                let env = self.status_env();
                let ctx = StatusCtx {
                    table: &self.table,
                    env: &env,
                };
                st::set_opr_status_count(&mut self.units[i], id, &ctx);
                if self.on_opr(i, id, rng) {
                    remove |= 1u64 << id;
                }
            } else {
                self.units[i].opr.insert(id, opr - coef);
            }
        }
        for id in ids_of(remove) {
            let env = self.status_env();
            remove_status(&mut self.units[i], id, &env);
        }
        // `ActiveTimeStatus` (`btl_stat.cs:531-543`).
        for id in ids_of(self.units[i].status_current & st::CONTI_COUNT) {
            let Some(c) = self.units[i].conti.get(&id).copied() else {
                continue;
            };
            let c = c - coef;
            self.units[i].conti.insert(id, c);
            if c < 0 {
                let env = self.status_env();
                if remove_status(&mut self.units[i], id, &env) == st::REMOVE_DONE {
                    self.emit(BattleEvent::StatusExpired {
                        unit: i,
                        status: id,
                    });
                }
            }
        }
        self.sync(&prev);
    }

    /// `OnOpr` of the five tick statuses (`DefaultStatus/PoisonStatusScript.cs`,
    /// `VenomStatusScript.cs`, `RegenStatusScript.cs`, `DoomStatusScript.cs`,
    /// `GradualPetrifyStatusScript.cs`). Returns the "remove me" flag.
    fn on_opr(&mut self, i: usize, id: u8, rng: &mut dyn BattleRandom) -> bool {
        let status = 1u64 << id;
        if status & (st::POISON | st::VENOM | st::REGEN) != 0 {
            self.emit(BattleEvent::StatusTick {
                unit: i,
                status: id,
            });
            let u = &mut self.units[i];
            if u.is_under_any_status(st::PETRIFY) {
                return false;
            }
            let mut amount = u.max_hp >> 4;
            let mut mp_amount = u.max_mp >> 4;
            if u.is_under_any_status(st::EASY_KILL) {
                amount >>= 2;
                mp_amount >>= 2;
            }
            if status == st::REGEN {
                if u.is_zombie() {
                    if u.cur_hp > amount {
                        u.set_current_hp(u.cur_hp - amount);
                    } else {
                        u.kill();
                    }
                } else {
                    u.set_current_hp((u.cur_hp + amount).min(u.max_hp));
                }
            } else {
                if status == st::VENOM {
                    u.cur_mp = u.cur_mp.saturating_sub(mp_amount);
                }
                if u.cur_hp > amount {
                    u.set_current_hp(u.cur_hp - amount);
                } else {
                    u.kill();
                }
            }
            return false;
        }
        if status == st::DOOM {
            self.units[i].doom_counter -= 1;
            self.emit(BattleEvent::StatusTick {
                unit: i,
                status: id,
            });
            if self.units[i].doom_counter > 0 {
                return false;
            }
            let env = self.status_env();
            let ctx = StatusCtx {
                table: &self.table,
                env: &env,
            };
            alter_status(&mut self.units[i], 8, false, &ctx, rng);
            return true;
        }
        if status == st::GRADUAL_PETRIFY {
            self.units[i].gradual_petrify_counter -= 1;
            self.emit(BattleEvent::StatusTick {
                unit: i,
                status: id,
            });
            if self.units[i].gradual_petrify_counter > 0 {
                return false;
            }
            let stone = self.cmds.slot_of(i, 2);
            if !self.cmds.is_using(stone, &self.running) {
                let env = self.status_env();
                let ctx = StatusCtx {
                    table: &self.table,
                    env: &env,
                };
                alter_status(&mut self.units[i], 0, false, &ctx, rng);
            }
            return true;
        }
        false
    }

    // -- ATB add ------------------------------------------------------------------------

    /// `HonoluluBattleMain.ProcessActiveTime` (`HonoluluBattleMain.cs:413-562`)
    /// at battle speed 0: a single pass (no turn-based re-loop).
    fn process_active_time(
        &mut self,
        hooks: &mut dyn BattleHooks,
        rng: &mut dyn BattleRandom,
    ) -> Result<(), BattleError> {
        let advance = self.atb_runs();
        let factor = atb::clamp_speed_factor(self.config.atb_speed_factor);
        for i in 0..self.units.len() {
            if !self.sched[i].in_list {
                continue;
            }
            let g = GaugeInput {
                max_atb: self.sched[i].max_atb,
                coef: self.sched[i].atb_coef,
                speed_factor: factor,
                advance,
                keep_rest_time: self.config.keep_rest_time,
                hp: self.units[i].cur_hp,
                sel_mode: self.sched[i].sel_mode,
                atb_active: self.sched[i].atb_active,
            };
            let step = atb::step_gauge(&mut self.units[i].cur_atb, g);
            if step != GaugeStep::Full {
                continue;
            }
            self.emit(BattleEvent::AtbFull { unit: i });
            if self.units[i].is_under_any_status(st::PREVENT_ATB_CONFIRM) {
                continue;
            }
            if self.units[i].is_player {
                let main = self.cmds.slot_of(i, 0);
                if self.cmds.is_using(main, &self.running) {
                    continue;
                }
                if self.player_auto_attack(i, hooks, rng)? {
                    self.sched[i].sel_mode = 1;
                } else if !self.ready_players.contains(&i) {
                    self.ready_players.push(i);
                    self.emit(BattleEvent::PlayerReady { unit: i });
                }
            } else if let Some(action) = hooks.enemy_atb(self, i) {
                // Source order: the request runs the AI (which may reject and
                // reset `sel_mode`), then a true return sets `sel_mode = 1`.
                self.set_enemy_command(i, action, hooks)?;
                self.sched[i].sel_mode = 1;
            }
        }
        Ok(())
    }

    /// The `IAutoAttackStatusScript.OnATB` loop (`HonoluluBattleMain.cs:502-516`):
    /// Confuse (id 10) then Berserk (id 11), the only two implementations.
    fn player_auto_attack(
        &mut self,
        i: usize,
        hooks: &mut dyn BattleHooks,
        rng: &mut dyn BattleRandom,
    ) -> Result<bool, BattleError> {
        for id in [10u8, 11u8] {
            if !self.units[i].is_under_any_status(1u64 << id) {
                continue;
            }
            if !self.units[i].can_use_attack_command {
                // `RemoveStatus(Target, Confuse|Berserk)` then `return false`.
                let prev = self.snapshot();
                let env = self.status_env();
                remove_status(&mut self.units[i], id, &env);
                self.sync(&prev);
                continue;
            }
            let views = self.views();
            let target = if id == 11 {
                // `GetRandomBtlID(0)`: a random enemy.
                targeting::get_random_btl_id(&views, false, false, rng)
            } else {
                // `GetRandomBtlID(random8() & 1)`.
                let side = rng.random8() & 1 != 0;
                targeting::get_random_btl_id(&views, side, false, rng)
            };
            let main = self.cmds.slot_of(i, 0);
            self.set_command(main, cmd::ATTACK, ABILITY_ATTACK, target, 0, false, hooks)?;
            return Ok(true);
        }
        Ok(false)
    }

    // -- command engine -----------------------------------------------------------------------

    /// `btl_cmd.CommandEngine` (`btl_cmd.cs:564-669`).
    fn command_engine(
        &mut self,
        hooks: &mut dyn BattleHooks,
        rng: &mut dyn BattleRandom,
    ) -> Result<(), BattleError> {
        self.run_command_from_queue(rng);
        let list = self.running.clone();
        for id in list {
            if !self.running.contains(&id) {
                continue;
            }
            match self.cmds.slots[id].mode {
                CmdMode::Inspection => {
                    let ok = self.check_command_condition(id, rng)
                        && self.check_target_condition(id, rng)
                        && self.check_mp_condition(id);
                    if !ok {
                        if let Some(r) = self.cmds.slots[id].regist {
                            let cmd_no = self.cmds.slots[id].cmd_no;
                            self.emit(BattleEvent::CommandRejected { unit: r, cmd_no });
                        }
                        self.cmds.slots[id].mode = CmdMode::Idle;
                    } else {
                        self.cmds.slots[id].mode = CmdMode::SelectVfx;
                    }
                }
                CmdMode::SelectVfx => {
                    // `CMD_MODE_SELECT_VFX` (`btl_cmd.cs:606-650`): confirm a
                    // target, try covers, confirm again, then the effect
                    // setup runs `CheckReflec`, then the item is used.
                    if !self.confirm_valid_target(id) {
                        self.cmds.slots[id].mode = CmdMode::Done;
                    } else {
                        let before = self.cmds.slots[id].tar_id;
                        self.try_cover(id);
                        let after = self.cmds.slots[id].tar_id;
                        if before != after {
                            self.emit(BattleEvent::Covered {
                                target_id: before & !after,
                                cover_id: after & !before,
                            });
                        }
                        if !self.confirm_valid_target(id) {
                            self.cmds.slots[id].mode = CmdMode::Done;
                        } else {
                            self.check_reflect(id, rng);
                            self.consume_item(id);
                            self.cmds.slots[id].mode = CmdMode::Loop;
                        }
                    }
                }
                CmdMode::Loop => {
                    self.command_loop(id, hooks, rng)?;
                }
                CmdMode::Idle => {
                    if self.cmds.slots[id].cmd_no != cmd::SYS_ESCAPE {
                        self.cmds.slots[id].mode = CmdMode::Done;
                    } else {
                        self.finish_command(id, rng)?;
                        continue;
                    }
                }
                CmdMode::Done => {}
            }
            if self.cmds.slots[id].mode == CmdMode::Done {
                self.finish_command(id, rng)?;
            }
        }
        Ok(())
    }

    /// `RunCommandFromQueue` at speed mode 0 (`btl_cmd.cs:488-562`).
    fn run_command_from_queue(&mut self, rng: &mut dyn BattleRandom) {
        if !self.running.is_empty() || self.cmds.queue.is_empty() {
            return;
        }
        let alive = self.any_player_alive();
        let units = &self.units;
        let cannot_act = |r: usize| units[r].is_under_any_status(st::CANNOT_ACT);
        let death = |r: usize| units[r].is_under_any_status(st::DEATH);
        let is_enemy = |r: usize| !units[r].is_player;
        let Some(id) = self.cmds.first_ready(alive, &cannot_act, &death, &is_enemy) else {
            return;
        };
        let cmd_no = self.cmds.slots[id].cmd_no;
        // `!IsNativeEnableAtb() && IsCommandDeclarable`.
        if !self.atb_runs() && is_declarable(cmd_no) {
            return;
        }
        if let Some(r) = self.cmds.slots[id].regist
            && !(cmd::ENEMY_REACTION..=cmd::BOUNDARY_UPPER_CHECK).contains(&cmd_no)
            && self.units[r].is_under_any_status(st::HEAT)
        {
            // Default `OnCommandRun` (`btl_cmd.cs:530-557`).
            if self.units[r].is_under_any_status(st::EASY_KILL) {
                self.units[r].set_current_hp(0);
                self.cmds.kill_command(id);
                return;
            }
            let prev = self.snapshot();
            let env = self.status_env();
            let ctx = StatusCtx {
                table: &self.table,
                env: &env,
            };
            let res = alter_status(&mut self.units[r], 8, false, &ctx, rng);
            self.sync(&prev);
            if res == st::AlterResult::Success {
                self.cmds.kill_command(id);
                return;
            }
        }
        self.running.push(id);
        self.cmds.kill_command(id);
        if let Some(r) = self.cmds.slots[id].regist {
            self.emit(BattleEvent::CommandStarted { unit: r, cmd_no });
        }
    }

    /// `CheckCommandCondition` (`btl_cmd.cs:939-1129`) for the parts that do
    /// not need UI, items, inventory or unported abilities.
    fn check_command_condition(&mut self, id: CmdId, rng: &mut dyn BattleRandom) -> bool {
        let Some(r) = self.cmds.slots[id].regist else {
            return true;
        };
        let cmd_no = self.cmds.slots[id].cmd_no;
        if self.dead(r) && (cmd_no <= cmd::ENEMY_COUNTER || cmd_no > cmd::BOUNDARY_UPPER_CHECK) {
            return false;
        }
        // `CheckMagicCondition` (`:1132`): Silence blocks category bit 2.
        let category = self.cmds.slots[id]
            .spec
            .as_ref()
            .map_or(0, |s| s.command.ability_category);
        if self.units[r].is_under_any_status(st::SILENCE) && category & 2 != 0 {
            return false;
        }
        if !self.item_available(id) {
            return false;
        }
        // `DecideMagicSword` (`btl_cmd.cs:1016-1021`).
        if let Some(set) = self.magic_sword_of(r, cmd_no, self.cmds.slots[id].sub_no)
            && !self.decide_magic_sword(r, &set)
        {
            self.emit(BattleEvent::CombinationFailed { unit: r });
            return false;
        }
        match cmd_no {
            cmd::SYS_STONE => {
                let prev = self.snapshot();
                let env = self.status_env();
                let ctx = StatusCtx {
                    table: &self.table,
                    env: &env,
                };
                alter_status(&mut self.units[r], 0, false, &ctx, rng);
                self.sync(&prev);
                false
            }
            cmd::SYS_TRANS => true,
            cmd::SYS_ESCAPE => {
                if self.phase != Phase::Normal {
                    return true;
                }
                for j in 0..self.units.len() {
                    if self.sched[j].in_list
                        && self.units[j].is_player
                        && !self.units[j].is_under_any_status(st::CANNOT_ESCAPE)
                        && self.units[j].cur_hp > 0
                    {
                        self.phase = Phase::MenuOff;
                        self.menu_off_seq = MenuOffSeq::Escape;
                    }
                }
                if self.phase == Phase::MenuOff && self.menu_off_seq == MenuOffSeq::Escape {
                    self.escape_count += 1;
                    if self.cmds.slots[id].sub_no == 0 {
                        self.ability_flee = true;
                    }
                    let removed = self.cmds.kill_all();
                    self.after_dequeue(&removed);
                    self.emit(BattleEvent::PhaseChanged {
                        phase: Phase::MenuOff,
                    });
                } else {
                    self.cmds.cmd_status &= 0xFFFE;
                }
                false
            }
            _ => true,
        }
    }

    /// `CheckTargetCondition` (`btl_cmd.cs:1221-1272`).
    fn check_target_condition(&mut self, id: CmdId, rng: &mut dyn BattleRandom) -> bool {
        let (tar, cursor, short, cmd_no) = {
            let s = &self.cmds.slots[id];
            (s.tar_id, s.cursor, s.short_range, s.cmd_no)
        };
        let for_dead = cmd_no == cmd::SYS_TRANS
            || self.cmds.slots[id]
                .spec
                .as_ref()
                .is_some_and(|s| s.for_dead);
        let views = self.views();
        match targeting::check_target_condition(&views, tar, cursor, short, for_dead, rng) {
            TargetCheck::Valid(t) => {
                self.cmds.slots[id].tar_id = t;
                true
            }
            TargetCheck::Invalid => false,
        }
    }

    /// `CheckMpCondition` + `ConsumeMp` (`btl_cmd.cs:1199-1219,1653`).
    fn check_mp_condition(&mut self, id: CmdId) -> bool {
        let Some(r) = self.cmds.slots[id].regist else {
            return true;
        };
        let Some(spec) = self.cmds.slots[id].spec.clone() else {
            return true;
        };
        let mut command = spec.command;
        command.command_id = self.cmds.slots[id].cmd_no;
        cost::try_consume_mp(
            &command,
            &mut self.units[r],
            false,
            self.sched[r].mp_cost_factor,
        )
    }

    /// `CMD_MODE_LOOP` (`btl_cmd.cs:653-656`, `CheckCommandLoop` `:1439`):
    /// apply the effect once the injected effect delay has passed, finish
    /// once the injected total has passed.
    fn command_loop(
        &mut self,
        id: CmdId,
        hooks: &mut dyn BattleHooks,
        rng: &mut dyn BattleRandom,
    ) -> Result<(), BattleError> {
        let t = self.config.timing;
        if !self.cmds.slots[id].effect_done
            && self.cmds.slots[id].loop_ticks >= t.effect_after_loop_ticks
        {
            self.cmds.slots[id].effect_done = true;
            self.apply_effect(id, hooks, rng)?;
        }
        if self.cmds.slots[id].effect_done && self.cmds.slots[id].loop_ticks >= t.loop_total_ticks {
            // `ReqFinishCommand` replays a partly reflected command on the
            // reflected targets instead of finishing.
            if !self.arm_reflect_second_pass(id) {
                self.cmds.slots[id].mode = CmdMode::Done;
            }
        }
        self.cmds.slots[id].loop_ticks += 1;
        Ok(())
    }

    /// `ExecVfxCommand` for every target (`btl_cmd.cs:1526-1583`): `SBattleCalculator.CalcMain`
    /// per target in `btl_list` order. System commands (`SysTrans`, jump) have
    /// no calculator step. Script 57 (Flee) runs `FleeScript.Perform`.
    fn apply_effect(
        &mut self,
        id: CmdId,
        hooks: &mut dyn BattleHooks,
        rng: &mut dyn BattleRandom,
    ) -> Result<(), BattleError> {
        let (cmd_no, sub_no, tar, cursor, short, regist) = {
            let s = &self.cmds.slots[id];
            (
                s.cmd_no,
                s.sub_no,
                s.tar_id,
                s.cursor,
                s.short_range,
                s.regist,
            )
        };
        if cmd_no == cmd::JUMP
            && let Some(caster) = regist
        {
            self.start_jump(caster, tar);
            return Ok(());
        }
        if matches!(cmd_no, cmd::JUMP | cmd::JUMP_IN_TRANCE | cmd::SYS_TRANS) {
            return Ok(());
        }
        let (Some(caster), Some(spec)) = (regist, self.cmds.slots[id].spec.clone()) else {
            return Ok(());
        };
        if spec.command.script_id == 57 {
            self.flee_ability(hooks)?;
            return Ok(());
        }
        if spec.command.script_id == 56 {
            return Err(BattleError::Unported("script 56 outside the flee key path"));
        }
        for t in 0..self.units.len() {
            if !self.sched[t].in_list || self.units[t].id & tar == 0 {
                continue;
            }
            let mut command: BattleCommand = spec.command.clone();
            command.command_id = cmd_no;
            command.target_id = tar;
            if self.cmds.slots[id].reflec_mode == 1 {
                command.reflect_target_ids = Some(self.cmds.slots[id].reflec_targets);
            }
            command.is_many_target = cursor & 1 != 0;
            command.is_short_range = short;
            if super::reactions::is_item_command(cmd_no) && self.units[caster].is_player {
                command.item_count = self.inventory.get(&sub_no).copied().unwrap_or(0);
            }
            let env = CalcEnv {
                escape_key_pressed: self.escape_key,
                battle_menu_enabled: self.menu_enabled,
                status: self.status_env(),
                ..CalcEnv::default()
            };
            let prev = self.snapshot();
            let outcome = calc_main(&mut self.units, caster, t, command, env, &self.table, rng)?;
            self.sync(&prev);
            if let Some(item) = outcome.stolen {
                if let Some(item) = item {
                    *self.inventory.entry(item as i32).or_insert(0) += 1;
                }
                self.emit(BattleEvent::Stole {
                    caster,
                    target: t,
                    item,
                });
            }
            if let Some(result) = outcome.eaten {
                self.emit(BattleEvent::Ate {
                    caster,
                    target: t,
                    result,
                });
            }
            let target_hp = self.units[t].cur_hp;
            self.emit(BattleEvent::EffectApplied {
                caster,
                target: t,
                target_hp,
            });
            self.after_effect(
                caster,
                t,
                (cmd_no, sub_no),
                spec.command.ability_category,
                &outcome,
                hooks,
                rng,
            )?;
        }
        Ok(())
    }

    /// Jump (`btl_cmd.cs:1025`, `JumpStatusScript.Apply` and
    /// `OnFinishCommand`): the caster takes the Jump status, leaves the
    /// field (`SetDisappear`, untargetable) and lands with Spear after
    /// `ContiCnt (10) * 4 * (60 - Will)` ATB ticks. Jump in Trance (its
    /// repeating Spear 2) is not ported.
    fn start_jump(&mut self, caster: usize, tar_id: u16) {
        let will = i32::from(self.units[caster].will);
        self.units[caster].status_current |= st::JUMP;
        self.sched[caster].targetable = false;
        self.jumps.insert(
            caster,
            Jump {
                countdown: 10 * 4 * (60 - will),
                target: tar_id,
                landing: false,
            },
        );
        self.emit(BattleEvent::Jumped { unit: caster });
    }

    /// `ProcessJumpSpear`: the countdown runs on the ATB clock and queues
    /// Spear (`cmd[1]`) once it passes zero.
    fn jump_countdown(&mut self, i: usize, hooks: &mut dyn BattleHooks) -> Result<(), BattleError> {
        let Some(jump) = self.jumps.get(&i).copied() else {
            return Ok(());
        };
        if jump.landing || !self.atb_runs() {
            return Ok(());
        }
        let countdown = jump.countdown - i32::from(self.sched[i].atb_coef);
        self.jumps.insert(i, Jump { countdown, ..jump });
        if countdown < 0 {
            let id = self.cmds.slot_of(i, 1);
            let cursor = u8::from(jump.target.count_ones() > 1);
            if self.set_command(id, cmd::SPEAR, 185, jump.target, cursor, false, hooks)? {
                self.jumps.insert(
                    i,
                    Jump {
                        landing: true,
                        ..jump
                    },
                );
            }
        }
        Ok(())
    }

    /// Spear landed: Jump ends (`RemoveStatus(Jump)` -> `Remove`: back on
    /// the field, gauge emptied).
    fn end_jump(&mut self, unit: usize) {
        if self.jumps.remove(&unit).is_some() {
            self.units[unit].status_current &= !st::JUMP;
            self.sched[unit].targetable = true;
            self.sched[unit].sel_mode = 0;
            if self.units[unit].is_player {
                self.units[unit].cur_atb = 0;
            }
            self.emit(BattleEvent::Landed { unit });
        }
    }

    /// `btl_cmd.FinishCommand` (`btl_cmd.cs:1289-1408`): gauge reset for a
    /// finished main command, trance decay, `CheckPointData`, slot cleared.
    fn finish_command(&mut self, id: CmdId, rng: &mut dyn BattleRandom) -> Result<(), BattleError> {
        let (cmd_no, regist, slot) = {
            let s = &self.cmds.slots[id];
            (s.cmd_no, s.regist, s.slot)
        };
        if cmd_no == cmd::SPEAR
            && let Some(r) = regist
        {
            self.end_jump(r);
        }
        if let Some(r) = regist {
            if is_declarable(cmd_no) {
                if slot == 0 && cmd_no != cmd::JUMP && cmd_no != cmd::JUMP_IN_TRANCE {
                    // `ResetCurrentBattlerActiveTime` (`:1421-1429`).
                    let max = self.sched[r].max_atb;
                    if self.config.keep_rest_time && max > 0 {
                        self.units[r].cur_atb = (self.units[r].cur_atb - max).max(0);
                    } else {
                        self.units[r].cur_atb = 0;
                    }
                    self.sched[r].sel_mode = 0;
                }
                self.finish_trance_decay(r, cmd_no)?;
            }
            self.ready_players.retain(|&u| u != r);
            self.check_point_data(r, rng);
        }
        self.finish_cover(id);
        self.running.retain(|&c| c != id);
        self.cmds.clear(id);
        if let Some(r) = regist {
            self.emit(BattleEvent::CommandFinished { unit: r, cmd_no });
        }
        Ok(())
    }

    /// `IsNeedToDecreaseTrance` and the decay (`btl_cmd.cs:1299-1338,1410-1419`).
    fn finish_trance_decay(&mut self, r: usize, cmd_no: u32) -> Result<(), BattleError> {
        let u = &self.units[r];
        if !u.is_under_any_status(st::TRANCE)
            || u.status_permanent & st::TRANCE != 0
            || cmd_no == cmd::JUMP
            || cmd_no == cmd::JUMP_IN_TRANCE
        {
            return Ok(());
        }
        if u.will == 0 {
            return Err(BattleError::DivideByZero(
                "trance decay (300 - level) / will",
            ));
        }
        let mut delta = (300 - i32::from(u.level)) / i32::from(u.will) * 10;
        if cmd_no == cmd::DOUBLE_WHITE_MAGIC || cmd_no == cmd::DOUBLE_BLACK_MAGIC {
            delta /= 2;
        }
        if delta >= 0 {
            if i32::from(u.trance) > delta {
                self.units[r].trance -= delta as u8;
            } else {
                let prev = self.snapshot();
                let env = self.status_env();
                remove_status(&mut self.units[r], 14, &env);
                self.sync_removal_only(&prev);
            }
        }
        Ok(())
    }

    /// Slot id helper for tests and callers.
    pub fn slot(&self, unit: usize, k: usize) -> CmdId {
        debug_assert!(k < SLOTS_PER_UNIT);
        self.cmds.slot_of(unit, k)
    }

    /// Initial gauges for a normal start (`btl_init.cs:430,494`), in unit
    /// order; one `random16` draw per unit.
    pub fn roll_initial_atb(
        &mut self,
        start: StartType,
        rng: &mut dyn BattleRandom,
    ) -> Result<(), BattleError> {
        for i in 0..self.units.len() {
            let max = self.sched[i].max_atb;
            let v = if self.units[i].is_player {
                atb::initial_player_atb(max, start, rng)
            } else {
                atb::initial_enemy_atb(max, start, rng)
            };
            self.units[i].cur_atb = v.ok_or(BattleError::DivideByZero(
                "random16 % max.at (btl_init.cs:430)",
            ))?;
        }
        Ok(())
    }
}

/// `Dex`-independent default so callers can build a `StatusTable`-free
/// battle for gauge-only checks.
pub fn empty_status_table() -> StatusTable {
    StatusTable::new(Vec::new())
}
