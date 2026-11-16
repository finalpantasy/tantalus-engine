//! Battle reactions and command effects layered on the B5 machine: Reflect,
//! Cover, Counter, Item commands and Double Cast.
//!
//! Sources, all in the pinned Memoria `c716e1fa` under `Assembly-CSharp/`:
//!
//! * Reflect: `btl_cmd.CheckReflec` (`Global/btl_cmd.cs:876-936`), the
//!   partial-reflect second pass `ReqFinishCommand` / `btl_vfx.LoopBattleVfxForReflect`
//!   (`btl_cmd.cs:1274-1281`, `Global/btl_vfx.cs:16-26`), and the damage
//!   multiplier `BattleCommand.GetReflectMultiplierOnTarget` (already in B4).
//! * Cover: support ability 37 `Command` feature
//!   (`Memoria.Patcher/StreamingAssets/Data/Characters/Abilities/AbilityFeatures.txt:761-764`)
//!   evaluated by `SupportingAbilityFeature.TriggerOnCommand`
//!   (`Memoria/Data/Characters/CharacterAbilityGems.cs:789`), then
//!   `btl_abil.CheckCoverAbility` / `FindStrongestDefender`
//!   (`FF9/btl_abil.cs:191-230`) called from `CMD_MODE_SELECT_VFX`
//!   (`btl_cmd.cs:606-625`), and the cleanup in `FinishCommand`
//!   (`btl_cmd.cs:1345-1360`).
//! * Counter: support ability 36 `Ability AsTarget` feature (`AbilityFeatures.txt:755-758`),
//!   `TriggerOnAbility` with `when = "EffectDone"` (`CharacterAbilityGems.cs:546`,
//!   `SBattleCalculator.cs:299-302`) and `btl_cmd.SetCounter` (`btl_cmd.cs:336`).
//! * Items: `btl_util.GetCommandItem` / `GetCommandScriptId`
//!   (`FF9/btl_util.cs:405-450`), `CheckCommandCondition` item count
//!   (`btl_cmd.cs:1001-1011`), `CMD_MODE_SELECT_VFX` `ItemUse`
//!   (`btl_cmd.cs:643-648`), `BattleHUD.ItemUse` / `ItemUnuse`
//!   (`BattleHUD.Public.cs:846-856`).
//! * Double Cast: the `ScriptId == 80` branch of `btl_cmd.SetCommand`
//!   (`btl_cmd.cs:162-241`).
//!
//! Support abilities are data in Memoria (`AbilityFeatures.txt`, evaluated by
//! NCalc). Only the default rows 36 and 37 are ported, as typed code; a mod
//! that changes the feature text is not modeled.

use super::command::{BattleCommand, ItemData};
use super::machine::{Battle, BattleError, BattleEvent, BattleHooks};
use super::queue::{CmdId, CommandSpec, cmd, is_declarable};
use super::rng::BattleRandom;
use super::status as st;
use super::targeting::{count_bits, random_id, target_type};

/// `SupportAbility.Counter` (`AbilityFeatures.txt:754`).
pub const SA_COUNTER: u8 = 36;
/// `SupportAbility.Cover` (`AbilityFeatures.txt:761`).
pub const SA_COVER: u8 = 37;
/// `SupportAbility.Eye4Eye`; read by the Counter condition (`AbilityFeatures.txt:757`).
pub const SA_EYE_4_EYE: u8 = 39;

/// `BattleAbilityId.Attack` as the Counter feature's `[code=Counter] 176`.
pub const COUNTER_ABILITY_ATTACK: i32 = 176;

/// `BattleStatusConst.Immobilized` = `CannotAct | Death | Jump`
/// (`Memoria/Data/Battle/BattleStatus.cs:128`).
pub const IMMOBILIZED: u64 = st::CANNOT_ACT | st::DEATH | st::JUMP;

/// `BattleStatusConst.PreventCounter` (`BattleStatus.cs:130`).
pub const PREVENT_COUNTER: u64 = st::CANNOT_ACT | st::DEATH | st::SLEEP;

/// `BattleStatusConst.PreventReflect` (`BattleStatus.cs:151`).
pub const PREVENT_REFLECT: u64 = st::PETRIFY;

impl Battle {
    /// `ConfirmValidTarget` (`btl_cmd.cs:761`): some target bit belongs to a
    /// listed, targetable unit.
    pub(super) fn confirm_valid_target(&self, id: CmdId) -> bool {
        let tar = self.cmds.slots[id].tar_id;
        (0..self.units.len()).any(|j| {
            self.sched[j].in_list && self.units[j].id & tar != 0 && self.sched[j].targetable
        })
    }

    // -- cover -----------------------------------------------------------------

    /// The `TriggerOnCommand` pass and the `CheckCoverAbility` loop of
    /// `CMD_MODE_SELECT_VFX` (`btl_cmd.cs:606-625`) for support ability 37.
    ///
    /// Feature condition (`AbilityFeatures.txt:762`): the cover user is not
    /// targeted, is on the single target's side, the command is `EnemyAtk`
    /// with category bit 8, the target is a single unit, and the cover user is
    /// not under Heat. The user must not be immobilized
    /// (`CharacterAbilityGems.cs:791`). `TryCover` (`:764`): the target is not
    /// Death or Petrify and is under Low HP.
    ///
    /// Not modeled: SA 38 Protect Girls (needs the character's female
    /// category), mod-edited feature text.
    pub(super) fn try_cover(&mut self, id: CmdId) {
        let (cmd_no, tar, caster, category) = {
            let s = &self.cmds.slots[id];
            (
                s.cmd_no,
                s.tar_id,
                s.regist,
                s.spec.as_ref().map_or(0, |x| x.command.ability_category),
            )
        };
        // `target != null` requires exactly one target bit
        // (`Comn.countBits(tar_id) == 1`, `CharacterAbilityGems.cs:803`).
        if caster.is_none() || count_bits(u64::from(tar)) != 1 {
            return;
        }
        let Some(target) =
            (0..self.units.len()).find(|&j| self.sched[j].in_list && self.units[j].id == tar)
        else {
            return;
        };
        let target_player = self.units[target].is_player;
        let target_status = self.units[target].status_current;
        let mut try_cover: u16 = 0;
        for u in 0..self.units.len() {
            if !self.sched[u].in_list || !self.units[u].has_support_ability(SA_COVER) {
                continue;
            }
            let unit = &self.units[u];
            if unit.is_under_any_status(IMMOBILIZED) {
                continue;
            }
            let condition = tar & unit.id == 0
                && target_player == unit.is_player
                && cmd_no == cmd::ENEMY_ATK
                && category & 8 != 0
                && !unit.is_under_any_status(st::HEAT);
            if condition
                && target_status & (st::DEATH | st::PETRIFY) == 0
                && target_status & st::LOW_HP != 0
            {
                try_cover |= unit.id;
            }
        }
        if try_cover == 0 {
            return;
        }
        let targets: Vec<usize> = (0..self.units.len())
            .filter(|&j| self.sched[j].in_list && self.units[j].id & tar != 0)
            .collect();
        for t in targets {
            let target_id = self.units[t].id;
            let cover_id = self.check_cover_ability(t, try_cover);
            if cover_id != 0 {
                let slot = &mut self.cmds.slots[id];
                slot.tar_id = (slot.tar_id & !target_id) | cover_id;
                slot.covered = true;
            }
        }
    }

    /// `btl_abil.CheckCoverAbility` (`btl_abil.cs:191-218`). Returns the
    /// covering unit's id, or 0. Busy test: `IsBtlBusy(CASTER | MAGIC_CASTER)`
    /// over the running commands; `magic_caster_id` (Magic Sword) is not
    /// modeled.
    pub(super) fn check_cover_ability(&mut self, target: usize, candidates: u16) -> u16 {
        let target_id = self.units[target].id;
        // A unit already covering this target keeps covering it.
        for u in 0..self.units.len() {
            if self.sched[u].in_list && self.units[u].cover_unit == Some(target_id) {
                return self.units[u].id;
            }
        }
        // `FindStrongestDefender` (`btl_abil.cs:220-232`): highest current HP,
        // the first unit wins a tie (`coverBy.CurrentHp < unit.CurrentHp`).
        let mut best: Option<usize> = None;
        for u in 0..self.units.len() {
            if !self.sched[u].in_list || self.units[u].id & candidates == 0 {
                continue;
            }
            if self.units[u].id == target_id || self.units[u].covering || self.running_caster(u) {
                continue;
            }
            if best.is_none_or(|b| self.units[b].cur_hp < self.units[u].cur_hp) {
                best = Some(u);
            }
        }
        let Some(b) = best else {
            return 0;
        };
        self.units[b].cover_unit = Some(target_id);
        self.units[b].covering = true;
        self.units[b].id
    }

    fn running_caster(&self, u: usize) -> bool {
        self.running
            .iter()
            .any(|&c| self.cmds.slots[c].regist == Some(u))
    }

    /// `FinishCommand` cover cleanup (`btl_cmd.cs:1345-1360`): a covering unit
    /// stops covering once no other running command still carries a cover on
    /// it.
    pub(super) fn finish_cover(&mut self, id: CmdId) {
        if !self.cmds.slots[id].covered {
            return;
        }
        let tar = self.cmds.slots[id].tar_id;
        for t in 0..self.units.len() {
            if !self.sched[t].in_list || self.units[t].id & tar == 0 || !self.units[t].covering {
                continue;
            }
            let tid = self.units[t].id;
            let other = self.running.iter().any(|&o| {
                o != id && self.cmds.slots[o].covered && self.cmds.slots[o].tar_id & tid != 0
            });
            if !other {
                self.units[t].cover_unit = None;
                self.units[t].covering = false;
            }
        }
    }

    // -- reflect ---------------------------------------------------------------

    /// `btl_cmd.CheckReflec` (`btl_cmd.cs:876-936`), run once per command when
    /// its effect is set up (`BTL_VFX_REQ.SetupVfxRequest`, `BTL_VFX_REQ.cs:55`).
    ///
    /// Applies to commands with category bit 1 that are not Item, AutoPotion
    /// or Throw and not `ReflectNull`. A listed, targetable target that is under
    /// Reflect (and not Petrify) reflects. Each reflecting player draws an
    /// enemy, each reflecting enemy draws a player, from the listed,
    /// targetable, non-Death units outside `tar_id` (`random8() % count`, first
    /// all party reflections then all enemy ones). If every target reflects
    /// the command turns into mode 1 aimed at the drawn units; otherwise mode 2
    /// keeps the non-reflecting targets and the drawn units get a second pass
    /// when the first ends. Returns the reflecting mask.
    ///
    /// Not modeled: `permanent_on_hold` statuses (`CurrentIncludeOnHold`),
    /// Memoria's `CharacterCommandType` of mod-defined Item or Throw commands,
    /// the four-unit cap of the source arrays.
    pub(super) fn check_reflect(&mut self, id: CmdId, rng: &mut dyn BattleRandom) -> u16 {
        let (cmd_no, tar, checked) = {
            let s = &self.cmds.slots[id];
            (s.cmd_no, s.tar_id, s.has_checked_reflect)
        };
        let Some(spec) = self.cmds.slots[id].spec.as_ref() else {
            return 0;
        };
        let item_type = matches!(cmd_no, cmd::ITEM | cmd::AUTO_POTION | cmd::THROW);
        if item_type
            || spec.command.ability_category & 1 == 0
            || spec.command.is_reflect_null
            || checked
        {
            return 0;
        }
        let mut reflecting: u16 = 0;
        let mut party_count = 0usize;
        let mut enemy_count = 0usize;
        let mut players: Vec<u16> = Vec::new();
        let mut enemies: Vec<u16> = Vec::new();
        for u in 0..self.units.len() {
            if !self.sched[u].in_list {
                continue;
            }
            let unit = &self.units[u];
            if unit.id & tar != 0 {
                if self.sched[u].targetable
                    && unit.status_current & st::APPLY_REFLECT != 0
                    && unit.status_current & PREVENT_REFLECT == 0
                {
                    reflecting |= unit.id;
                    if unit.is_player {
                        party_count += 1;
                    } else {
                        enemy_count += 1;
                    }
                }
            } else if !unit.is_under_any_status(st::DEATH) && self.sched[u].targetable {
                if unit.is_player {
                    players.push(unit.id);
                } else {
                    enemies.push(unit.id);
                }
            }
        }
        if party_count != 0 || enemy_count != 0 {
            let mut list: Vec<u16> = Vec::new();
            if !enemies.is_empty() {
                for _ in 0..party_count {
                    let k = rng.random8() as usize % enemies.len();
                    list.push(enemies[k]);
                }
            }
            if !players.is_empty() {
                for _ in 0..enemy_count {
                    let k = rng.random8() as usize % players.len();
                    list.push(players[k]);
                }
            }
            let mut targets = [0u16; 4];
            for (i, t) in targets.iter_mut().enumerate() {
                *t = list.get(i).copied().unwrap_or(0);
            }
            let slot = &mut self.cmds.slots[id];
            slot.reflec_targets = targets;
            if reflecting == tar {
                slot.reflec_mode = 1;
                slot.tar_id = merge_reflec(&targets);
            } else {
                slot.reflec_mode = 2;
                slot.tar_id = tar & !reflecting;
            }
            let caster = slot.regist;
            let mode = slot.reflec_mode;
            if let Some(caster) = caster {
                self.emit(BattleEvent::Reflected {
                    caster,
                    reflecting,
                    targets,
                    mode,
                });
            }
        }
        self.cmds.slots[id].has_checked_reflect = true;
        reflecting
    }

    /// `ReqFinishCommand` for `reflec == 2` without the SFX rework
    /// (`btl_cmd.cs:1276-1279`, `LoopBattleVfxForReflect` `btl_vfx.cs:16`): the
    /// command targets the reflected units and runs its loop again. Returns
    /// true when a second pass was armed.
    pub(super) fn arm_reflect_second_pass(&mut self, id: CmdId) -> bool {
        let slot = &mut self.cmds.slots[id];
        if slot.reflec_mode != 2 {
            return false;
        }
        slot.tar_id = merge_reflec(&slot.reflec_targets);
        slot.reflec_mode = 1;
        slot.effect_done = false;
        slot.loop_ticks = 0;
        true
    }

    // -- counter -----------------------------------------------------------------

    /// Support ability 36 after an effect on `target` (`TriggerOnAbility(...,
    /// "EffectDone", asTarget = true)`, `SBattleCalculator.cs:301`).
    ///
    /// Condition (`AbilityFeatures.txt:757`): the target can move; the command
    /// is declarable; caster and target are on opposite sides; category bit 8;
    /// and `Spirit (x2 with Eye 4 Eye) > GetRandom(0, 100)`, where `GetRandom`
    /// is `UnityEngine.Random.Range` (`NCalcUtility.cs:107`), one
    /// `random_range(0, 100)` draw drawn only when the earlier terms hold
    /// (NCalc `&&` short-circuits). Result: `SetCounter(target, Counter, 176,
    /// caster)`.
    pub(super) fn trigger_counter(
        &mut self,
        caster: usize,
        target: usize,
        cmd_no: u32,
        category: u8,
        hooks: &mut dyn BattleHooks,
        rng: &mut dyn BattleRandom,
    ) -> Result<(), BattleError> {
        let unit = &self.units[target];
        if !unit.has_support_ability(SA_COUNTER) || unit.is_under_any_status(IMMOBILIZED) {
            return Ok(());
        }
        if !(is_declarable(cmd_no)
            && self.units[caster].is_player != unit.is_player
            && category & 8 != 0)
        {
            return Ok(());
        }
        let spirit = i32::from(unit.will);
        let spirit = if unit.has_support_ability(SA_EYE_4_EYE) {
            2 * spirit
        } else {
            spirit
        };
        if spirit <= rng.random_range(0, 100) {
            return Ok(());
        }
        let caster_id = self.units[caster].id;
        self.set_counter(
            target,
            cmd::COUNTER,
            COUNTER_ABILITY_ATTACK,
            caster_id,
            hooks,
        )?;
        Ok(())
    }

    /// `btl_cmd.SetCounter` (`btl_cmd.cs:336-344`): refused while the unit is
    /// under `PreventCounter` or the battle is not in the normal phase;
    /// otherwise a priority command in slot 1 with `cursor` 1 when several
    /// targets. Returns false when nothing was queued. The monster-transform
    /// check is not modeled.
    pub fn set_counter(
        &mut self,
        unit: usize,
        command_id: u32,
        sub_no: i32,
        tar_id: u16,
        hooks: &mut dyn BattleHooks,
    ) -> Result<bool, BattleError> {
        if self.units[unit].is_under_any_status(PREVENT_COUNTER)
            || self.phase != super::machine::Phase::Normal
        {
            return Ok(false);
        }
        let id = self.cmds.slot_of(unit, 1);
        let cursor = u8::from(count_bits(u64::from(tar_id)) > 1);
        self.set_command(id, command_id, sub_no, tar_id, cursor, true, hooks)
    }

    // -- items -------------------------------------------------------------------

    /// The `CommandSpec` of an Item command: the base spec the hooks give for
    /// the command (the `Void` action row) with the script id and item data
    /// of `ITEM_DATA` (`GetCommandScriptId`, `btl_util.cs:445-446`;
    /// `BattleCommand.Item`, `BattleCommand.cs:24-28`) and the item's
    /// `info.ForDead` (`btl_cmd.cs:1231-1232`). Only player casters use items
    /// (`GetCommandItem`, `btl_util.cs:407`).
    pub(super) fn item_spec(
        &mut self,
        base: Option<CommandSpec>,
        cmd_no: u32,
        sub_no: i32,
        hooks: &mut dyn BattleHooks,
    ) -> Result<CommandSpec, BattleError> {
        let Some(ItemData {
            effect,
            script_id,
            for_dead,
        }) = hooks.item(sub_no)
        else {
            return Err(BattleError::UnknownCommand { cmd_no, sub_no });
        };
        let (mut command, default_ally) = match base {
            Some(s) => (s.command, s.default_ally),
            None => (BattleCommand::new(0), false),
        };
        command.script_id = script_id;
        command.item = Some(effect);
        Ok(CommandSpec {
            command,
            default_ally,
            for_dead,
        })
    }

    /// Item count check in `CheckCommandCondition` (`btl_cmd.cs:1001-1007`):
    /// an Item command with no copy left is rejected.
    pub(super) fn item_available(&self, id: CmdId) -> bool {
        let s = &self.cmds.slots[id];
        let Some(r) = s.regist else {
            return true;
        };
        if s.cmd_no != cmd::ITEM || !self.units[r].is_player {
            return true;
        }
        self.inventory.get(&s.sub_no).copied().unwrap_or(0) > 0
    }

    /// `UIManager.Battle.ItemUse` at `CMD_MODE_SELECT_VFX` (`btl_cmd.cs:643-648`,
    /// `FF9Item_Remove(id, 1)`). `ItemUnuse` does nothing, so a command that
    /// is cancelled before this point costs no item.
    pub(super) fn consume_item(&mut self, id: CmdId) {
        let s = &self.cmds.slots[id];
        let (Some(r), cmd_no, item) = (s.regist, s.cmd_no, s.sub_no) else {
            return;
        };
        if cmd_no != cmd::ITEM || !self.units[r].is_player {
            return;
        }
        if let Some(n) = self.inventory.get_mut(&item)
            && *n > 0
        {
            *n -= 1;
            let remaining = *n;
            self.emit(BattleEvent::ItemUsed {
                unit: r,
                item,
                remaining,
            });
        }
    }

    // -- double cast ---------------------------------------------------------------

    /// The `ScriptId == 80` branch of `btl_cmd.SetCommand` (`btl_cmd.cs:162-241`).
    /// `spec` is the Double Cast command's own action: its `power` is the
    /// first ability id and its `hit_rate` the second. Returns `(first,
    /// second)` target masks. `rng` feeds `Comn.randomID` in the branches that
    /// pick a random player or enemy.
    pub(super) fn double_cast_targets(
        first: &CommandSpec,
        second: &CommandSpec,
        caster_id: u16,
        tar_id: u16,
        rng: Option<&mut dyn BattleRandom>,
    ) -> Result<(u16, u16), BattleError> {
        let ft = first.command.target_type;
        let sd = second.command.target_type;
        let mut rng = rng;
        let mut pick = |mask: u32| -> Result<u16, BattleError> {
            match rng.as_deref_mut() {
                Some(r) => Ok(random_id(mask, r) as u16),
                None => Err(BattleError::Unported(
                    "double cast random target needs set_command_with_rng",
                )),
            }
        };
        let all_ally = |t: u8| t == target_type::ALL_ALLY || t == target_type::RANDOM_ALLY;
        let all_enemy = |t: u8| t == target_type::ALL_ENEMY || t == target_type::RANDOM_ENEMY;
        let pair = if all_ally(ft) {
            (0x0F, tar_id)
        } else if all_enemy(ft) {
            (0xF0, tar_id)
        } else if ft == target_type::EVERYONE {
            (0xFF, tar_id)
        } else if ft == target_type::SELF {
            (caster_id, tar_id)
        } else if all_ally(sd) {
            (tar_id, 0x0F)
        } else if all_enemy(sd) {
            (tar_id, 0xF0)
        } else if sd == target_type::EVERYONE {
            (tar_id, 0xFF)
        } else if sd == target_type::SELF {
            (tar_id, caster_id)
        } else if !first.default_ally && tar_id & 0xF0 == tar_id {
            let second_tar = if !second.default_ally {
                tar_id
            } else if sd == target_type::MANY_ALLY || sd == target_type::MANY_ANY {
                0x0F
            } else if sd == target_type::MANY_ENEMY {
                0xF0
            } else {
                pick(0x0F)?
            };
            (tar_id, second_tar)
        } else {
            let first_tar = if first.default_ally {
                tar_id
            } else if ft == target_type::MANY_ALLY {
                0x0F
            } else if ft == target_type::MANY_ENEMY || ft == target_type::MANY_ANY {
                0xF0
            } else {
                pick(0xF0)?
            };
            (first_tar, tar_id)
        };
        Ok(pair)
    }

    /// Queue both halves of a Double Cast: the first spell in slot 3 with the
    /// Double Cast MP cost as `CustomMPCost`, the second in slot 0 with a
    /// cost of 0 (`btl_cmd.cs:229-239`). `first_ability` is `Power`,
    /// `second_ability` is `HitRate` of the Double Cast action.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn queue_double_cast(
        &mut self,
        id: CmdId,
        caster: usize,
        double_spec: &CommandSpec,
        cmd_no: u32,
        tar_id: u16,
        hooks: &mut dyn BattleHooks,
        rng: Option<&mut dyn BattleRandom>,
    ) -> Result<bool, BattleError> {
        let first_id = double_spec.command.power;
        let second_id = double_spec.command.hit_rate;
        let first = hooks.spec(cmd_no, first_id, &self.units[caster]);
        let second = hooks.spec(cmd_no, second_id, &self.units[caster]);
        let (Some(first), Some(second)) = (first, second) else {
            return Err(BattleError::UnknownCommand {
                cmd_no,
                sub_no: first_id,
            });
        };
        let caster_id = self.units[caster].id;
        let (first_tar, second_tar) =
            Self::double_cast_targets(&first, &second, caster_id, tar_id, rng)?;
        let first_cursor = u8::from(count_bits(u64::from(first_tar)) > 1);
        let second_cursor = u8::from(count_bits(u64::from(second_tar)) > 1);
        let first_slot = self.cmds.slot_of(caster, 3);
        self.sched[caster].sel_mode = 1;
        let mp = double_spec.command.mp_cost;
        let ok1 = self.set_command_inner(
            first_slot,
            cmd_no,
            first_id,
            first_tar,
            first_cursor,
            false,
            Some(mp),
            hooks,
            None,
        )?;
        let ok2 = self.set_command_inner(
            id,
            cmd_no,
            second_id,
            second_tar,
            second_cursor,
            false,
            Some(0),
            hooks,
            None,
        )?;
        Ok(ok1 && ok2)
    }
}

/// `btl_cmd.MergeReflecTargetID` (`btl_cmd.cs:1431`).
pub fn merge_reflec(targets: &[u16; 4]) -> u16 {
    targets.iter().fold(0, |a, t| a | t)
}
