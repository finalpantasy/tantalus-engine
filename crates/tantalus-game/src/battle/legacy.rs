//! Legacy battle adapter: run the ported ATB machine
//! (`tantalus_legacy::battle::machine::Battle`) behind the coordinator's
//! ruleset surface, so a field encounter drives the real engine instead of
//! the synthetic action-timeline ruleset.
//!
//! Scope of this slice: player commands map onto machine commands through a
//! caller-provided action catalog; enemy AI is not driven (the `NoEnemyAi`
//! hook returns no action), so tests progress by player attacks. Battle
//! rewards stay caller-driven through [`super::bridge`] — the machine does
//! not yet invoke `SetBonus` on deaths.

use super::{BattleOutcome, CombatEvent, CombatInput, CombatRuleset, CombatUpdate, GameServices};
use std::collections::BTreeMap;
use tantalus_contracts::{ContentId, EngineError, EntityId, ErrorCategory};
use tantalus_legacy::battle::machine::{Battle as LegacyBattle, BattleHooks, EnemyAction, result};
use tantalus_legacy::battle::queue::{CommandSpec, cmd};
use tantalus_legacy::battle::rng::BattleRandom;
use tantalus_legacy::battle::targeting::target_type;
use tantalus_legacy::battle::{BattleCommand, BattleUnit};

fn battle_error(context: impl Into<String>, recoverable: bool) -> EngineError {
    EngineError::new(
        ErrorCategory::Conflict,
        context,
        "tantalus-game/legacy-battle",
        recoverable,
    )
}

/// A minimal command catalog for the adapter: Attack (calculator script 1,
/// single enemy) and Defend (script 103, self) — the same stand-in the
/// machine's own tests use. Commands outside the catalog have no calculator
/// effect (`None`), matching `btl_util.GetCommandAction`.
pub fn basic_spec(cmd_no: u32) -> Option<CommandSpec> {
    match cmd_no {
        cmd::ATTACK | cmd::ENEMY_ATK => {
            let mut command = BattleCommand::new(1);
            command.ability_category = 8;
            command.target_type = target_type::SINGLE_ENEMY;
            Some(CommandSpec {
                command,
                default_ally: false,
                for_dead: false,
            })
        }
        cmd::DEFEND => {
            let mut command = BattleCommand::new(103);
            command.hit_rate = 255;
            command.target_type = target_type::SELF;
            Some(CommandSpec {
                command,
                default_ally: true,
                for_dead: false,
            })
        }
        _ => None,
    }
}

/// Default hooks: the minimal Attack/Defend catalog and no enemy AI
/// (`enemy_atb` answers nothing). Also the natural `inner` hooks for the
/// script-driven [`tantalus_legacy::battle::script_host::EnemyAiHooks`].
pub struct NoEnemyAi;

impl BattleHooks for NoEnemyAi {
    fn spec(&mut self, cmd_no: u32, _sub_no: i32, _caster: &BattleUnit) -> Option<CommandSpec> {
        basic_spec(cmd_no)
    }

    fn enemy_atb(&mut self, _battle: &LegacyBattle, _unit: usize) -> Option<EnemyAction> {
        None
    }
}

/// The ruleset a coordinator runs: the synthetic action-timeline ruleset or
/// the legacy ATB machine behind [`LegacyBattleAdapter`].
#[derive(Debug)]
pub enum AnyRuleset {
    Synthetic(CombatRuleset),
    Legacy(LegacyBattleAdapter),
}

impl AnyRuleset {
    pub fn advance(
        &mut self,
        input: &CombatInput,
        services: &mut dyn GameServices,
    ) -> Result<CombatUpdate, EngineError> {
        match self {
            Self::Synthetic(ruleset) => ruleset.advance(input, services),
            Self::Legacy(adapter) => adapter.advance(input, services),
        }
    }
}

impl From<CombatRuleset> for AnyRuleset {
    fn from(ruleset: CombatRuleset) -> Self {
        Self::Synthetic(ruleset)
    }
}

impl From<LegacyBattleAdapter> for AnyRuleset {
    fn from(adapter: LegacyBattleAdapter) -> Self {
        Self::Legacy(adapter)
    }
}

/// Routes the machine's random draws through the simulation's RNG
/// authority (`GameServices::random_below`) so a battle replays from world
/// state alone. Draws are byte-granular: the field world advances an 8-bit
/// random source, so `random16` and `random_unit` carry 8 bits of entropy
/// until the world RNG widens.
struct ServicesRandom<'a> {
    services: &'a mut dyn GameServices,
}

impl BattleRandom for ServicesRandom<'_> {
    fn random16(&mut self) -> i32 {
        i32::try_from(self.services.random_below(65_536)).unwrap_or(i32::MAX)
    }

    fn random8(&mut self) -> i32 {
        i32::try_from(self.services.random_below(256)).unwrap_or(i32::MAX)
    }

    fn random_range(&mut self, min: i32, max_exclusive: i32) -> i32 {
        let span = max_exclusive.saturating_sub(min);
        if span <= 0 {
            return min;
        }
        min + i32::try_from(
            self.services
                .random_below(u32::try_from(span).unwrap_or(u32::MAX)),
        )
        .unwrap_or(min)
    }

    fn random_unit(&mut self) -> f32 {
        self.services.random_below(256) as f32 / 256.0
    }
}

/// Drives one legacy machine battle. Player commands arrive as
/// [`CombatInput`] values; the caller supplies the content-identity mappings
/// the machine cannot know: action id -> `(cmd_no, sub_no)`, field entity ->
/// machine unit index, and target entity -> `btl_id` bit for target masks.
/// Random draws route through the caller's `GameServices`, keeping the whole
/// simulation deterministic from world state.
pub struct LegacyBattleAdapter {
    battle: LegacyBattle,
    hooks: Box<dyn BattleHooks>,
    action_map: BTreeMap<ContentId, (u32, i32)>,
    unit_map: BTreeMap<EntityId, usize>,
    target_bits: BTreeMap<EntityId, u16>,
    outcome: Option<BattleOutcome>,
    /// Read position into `battle.log` for event surfacing.
    log_cursor: usize,
}

impl std::fmt::Debug for LegacyBattleAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LegacyBattleAdapter")
            .field("units", &self.battle.units.len())
            .field("outcome", &self.outcome)
            .finish()
    }
}

impl LegacyBattleAdapter {
    pub fn new(
        battle: LegacyBattle,
        action_map: BTreeMap<ContentId, (u32, i32)>,
        unit_map: BTreeMap<EntityId, usize>,
        target_bits: BTreeMap<EntityId, u16>,
    ) -> Self {
        Self {
            battle,
            hooks: Box::new(NoEnemyAi),
            action_map,
            unit_map,
            target_bits,
            outcome: None,
            log_cursor: 0,
        }
    }

    /// Replaces the hooks (e.g. with `EnemyAiHooks` driving enemy AI from
    /// decoded battle scripts).
    pub fn with_hooks(mut self, hooks: impl BattleHooks + 'static) -> Self {
        self.hooks = Box::new(hooks);
        self
    }

    pub fn battle(&self) -> &LegacyBattle {
        &self.battle
    }

    pub fn into_battle(self) -> LegacyBattle {
        self.battle
    }

    pub fn outcome(&self) -> Option<BattleOutcome> {
        self.outcome
    }

    /// Maps machine log entries recorded since the last call onto the
    /// coordinator's event surface. v1 surfaces death and completion
    /// notifications (the ones the reward loop and battle UI consume);
    /// command/action events stay machine-internal until a caller needs
    /// them — `ActionStarted` requires a content identity the adapter's
    /// one-way action map cannot always invert.
    pub fn take_events(&mut self) -> Vec<CombatEvent> {
        let mut events = Vec::new();
        let drained: Vec<_> = self.battle.log[self.log_cursor..].to_vec();
        self.log_cursor = self.battle.log.len();
        for (_, event) in drained {
            match event {
                tantalus_legacy::battle::machine::BattleEvent::UnitDied { unit } => {
                    let entity = self
                        .unit_map
                        .iter()
                        .find(|(_, index)| **index == unit)
                        .map(|(entity, _)| *entity);
                    if let Some(entity) = entity {
                        events.push(CombatEvent::UnitDefeated { unit: entity });
                    }
                }
                tantalus_legacy::battle::machine::BattleEvent::Result { result } => {
                    let outcome = match result {
                        result::VICTORY | result::VICTORY_NO_POSE => BattleOutcome::Victory,
                        result::DEFEAT => BattleOutcome::Defeat,
                        _ => continue,
                    };
                    events.push(CombatEvent::BattleCompleted { outcome });
                }
                _ => {}
            }
        }
        events
    }

    /// Advances the machine one tick, queueing a player command first when
    /// the input carries one and the unit is ready (`ReadyQueue`).
    pub fn advance(
        &mut self,
        input: &CombatInput,
        _services: &mut dyn GameServices,
    ) -> Result<CombatUpdate, EngineError> {
        if let Some(command) = &input.command {
            let Some(&(cmd_no, sub_no)) = self.action_map.get(&command.action) else {
                return Err(battle_error(
                    format!("no legacy mapping for action `{}`", command.action.as_str()),
                    true,
                ));
            };
            let Some(&unit) = self.unit_map.get(&command.actor) else {
                return Err(battle_error("commanding actor is not a battle unit", true));
            };
            if self.battle.ready_players.contains(&unit) {
                let tar_id = command.targets.iter().fold(0u16, |mask, target| {
                    mask | self.target_bits.get(target).copied().unwrap_or(0)
                });
                self.battle
                    .player_command(unit, cmd_no, sub_no, tar_id, 0, &mut *self.hooks)
                    .map_err(|error| {
                        battle_error(format!("legacy command rejected: {error:?}"), false)
                    })?;
            }
        }
        let mut rng = ServicesRandom {
            services: _services,
        };
        self.battle
            .tick(&mut *self.hooks, &mut rng)
            .map_err(|error| {
                battle_error(format!("legacy battle tick failed: {error:?}"), false)
            })?;
        if self.battle.over && self.outcome.is_none() {
            self.outcome = Some(match self.battle.result {
                Some(result::VICTORY) | Some(result::VICTORY_NO_POSE) => BattleOutcome::Victory,
                Some(result::DEFEAT) => BattleOutcome::Defeat,
                Some(result::ESCAPE) => BattleOutcome::Escape,
                Some(result::ENEMY_FLEE) => BattleOutcome::EnemyFlee,
                other => {
                    return Err(battle_error(
                        format!("unmapped legacy battle result {other:?}"),
                        false,
                    ));
                }
            });
        }
        Ok(CombatUpdate {
            tick: input.tick,
            events: Vec::new(),
            markers: Vec::new(),
            outcome: self.outcome,
            rewards: None,
            active_action: false,
        })
    }
}

/// Build the unit index and target-bit mappings from machine units whose
/// `id` is the `btl_id` bit, keyed by the field entity the caller assigns.
pub fn unit_mappings(
    pairs: &[(EntityId, u16)],
) -> (BTreeMap<EntityId, usize>, BTreeMap<EntityId, u16>) {
    let unit_map = pairs
        .iter()
        .enumerate()
        .map(|(index, (entity, _))| (*entity, index))
        .collect();
    let target_bits = pairs.iter().copied().collect();
    (unit_map, target_bits)
}
