use serde::{Deserialize, Serialize};
use tantalus_contracts::{ContentId, EntityId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Team {
    Player,
    Enemy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatUnit {
    pub id: EntityId,
    pub team: Team,
    pub level: i32,
    pub magic: i32,
    pub magic_defense: i32,
    pub hp: u32,
    pub max_hp: u32,
    pub mp: u32,
    pub max_mp: u32,
    pub alive: bool,
    pub reward: super::RewardBundle,
}

impl CombatUnit {
    pub fn new(id: EntityId, team: Team, hp: u32, mp: u32) -> Self {
        Self {
            id,
            team,
            level: 1,
            magic: 1,
            magic_defense: 0,
            hp,
            max_hp: hp,
            mp,
            max_mp: mp,
            alive: hp > 0,
            reward: super::RewardBundle::default(),
        }
    }
}

use super::*;
use std::collections::{BTreeMap, BTreeSet};
use tantalus_contracts::{ActionMarkerKind, EngineError, ErrorCategory, SemanticMarker, Tick};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BattleStatus {
    Running,
    Victory,
    Defeat,
}

pub trait GameServices {
    fn random_below(&mut self, upper: u32) -> u32;
    fn commit_rewards(&mut self, rewards: &RewardBundle) -> Result<(), EngineError>;
}

#[derive(Debug, Clone)]
pub struct CombatRuleset {
    actions: BTreeMap<ContentId, ActionDefinition>,
    units: BTreeMap<EntityId, CombatUnit>,
    active: Option<ActiveAction>,
    status: BattleStatus,
    tick: Tick,
    next_resolution: u64,
}

impl CombatRuleset {
    pub fn new(
        actions: Vec<ActionDefinition>,
        units: Vec<CombatUnit>,
    ) -> Result<Self, EngineError> {
        let mut action_map = BTreeMap::new();
        for action in actions {
            validate_action(&action)?;
            if action_map.insert(action.id.clone(), action).is_some() {
                return Err(battle_error("duplicate action definition", true));
            }
        }
        let mut unit_map = BTreeMap::new();
        for unit in units {
            if unit_map.insert(unit.id, unit).is_some() {
                return Err(battle_error("duplicate combat unit", true));
            }
        }
        Ok(Self {
            actions: action_map,
            units: unit_map,
            active: None,
            status: BattleStatus::Running,
            tick: Tick::default(),
            next_resolution: 1,
        })
    }

    pub fn status(&self) -> BattleStatus {
        self.status
    }

    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn unit(&self, id: EntityId) -> Option<&CombatUnit> {
        self.units.get(&id)
    }

    pub fn unit_mut(&mut self, id: EntityId) -> Option<&mut CombatUnit> {
        self.units.get_mut(&id)
    }

    pub fn advance(
        &mut self,
        input: &CombatInput,
        context: &mut dyn GameServices,
    ) -> Result<CombatUpdate, EngineError> {
        let mut staged = self.clone();
        let result = staged.advance_inner(input, context)?;
        *self = staged;
        Ok(result)
    }

    fn advance_inner(
        &mut self,
        input: &CombatInput,
        context: &mut dyn GameServices,
    ) -> Result<CombatUpdate, EngineError> {
        if input.stride == 0 {
            return Err(battle_error("combat stride must be nonzero", true));
        }
        let expected = self.tick.checked_add(input.stride).ok_or_else(|| {
            EngineError::new(
                ErrorCategory::CapacityExhausted,
                "combat tick overflow",
                "tantalus-game/battle",
                false,
            )
        })?;
        if input.tick != expected {
            return Err(battle_error(
                format!("expected {expected}, received {}", input.tick),
                true,
            ));
        }

        let mut update = CombatUpdate {
            tick: input.tick,
            events: Vec::new(),
            markers: Vec::new(),
            outcome: None,
            rewards: None,
            active_action: false,
        };
        if self.status != BattleStatus::Running {
            self.tick = input.tick;
            return Ok(update);
        }

        if let Some(actor) = input.interrupt {
            let Some(active) = self.active.take() else {
                return Err(battle_error("no active action can be interrupted", true));
            };
            if active.actor != actor {
                self.active = Some(active);
                return Err(battle_error(
                    "interruption actor does not own the active action",
                    true,
                ));
            }
            update.events.push(CombatEvent::ActionInterrupted {
                actor,
                action: active.definition.id,
            });
        }

        if self.active.is_some() {
            self.advance_active(input.stride, context, &mut update)?;
        }
        if self.status == BattleStatus::Running
            && self.active.is_none()
            && let Some(command) = &input.command
        {
            self.start_action(command.clone(), &mut update)?;
        }
        self.tick = input.tick;
        update.active_action = self.active.is_some();
        Ok(update)
    }

    fn start_action(
        &mut self,
        command: BattleCommand,
        update: &mut CombatUpdate,
    ) -> Result<(), EngineError> {
        let actor =
            self.units.get(&command.actor).cloned().ok_or_else(|| {
                battle_error(format!("unknown action actor {}", command.actor), true)
            })?;
        if !actor.alive {
            return Err(battle_error("a defeated actor cannot act", true));
        }
        let action =
            self.actions.get(&command.action).cloned().ok_or_else(|| {
                battle_error(format!("unknown action `{}`", command.action), true)
            })?;
        if actor.mp < action.mp_cost {
            return Err(battle_error("not enough MP for action", true));
        }
        if command.targets.is_empty() {
            return Err(battle_error("action requires at least one target", true));
        }
        let mut unique = BTreeSet::new();
        for target in &command.targets {
            if !unique.insert(*target) {
                return Err(battle_error("duplicate action target", true));
            }
            if !self.units.get(target).is_some_and(|unit| unit.alive) {
                return Err(battle_error("target is missing or already defeated", true));
            }
        }

        self.units.get_mut(&command.actor).unwrap().mp -= action.mp_cost;
        self.active = Some(ActiveAction {
            actor: command.actor,
            definition: action.clone(),
            targets: command.targets,
            elapsed_ticks: 0,
            next_marker: 0,
        });
        update.events.push(CombatEvent::ActionStarted {
            actor: command.actor,
            action: action.id,
        });
        Ok(())
    }

    fn advance_active(
        &mut self,
        stride: u32,
        context: &mut dyn GameServices,
        update: &mut CombatUpdate,
    ) -> Result<(), EngineError> {
        let mut active = self.active.take().expect("active action exists");
        let mut completed = false;
        active.elapsed_ticks = active
            .elapsed_ticks
            .saturating_add(stride)
            .min(active.definition.timeline.duration_ticks);

        while active.next_marker < active.definition.timeline.markers.len() {
            let marker = &active.definition.timeline.markers[active.next_marker];
            if marker.tick_offset > active.elapsed_ticks {
                break;
            }
            active.next_marker += 1;
            update.markers.push(SemanticMarker {
                tick: self.tick.checked_add(stride).unwrap_or(self.tick),
                action: active.definition.id.clone(),
                kind: marker.kind.clone(),
            });
            match &marker.kind {
                ActionMarkerKind::Hit => self.resolve_hits(&active, context, update)?,
                ActionMarkerKind::Complete => {
                    completed = true;
                    update.events.push(CombatEvent::ActionCompleted {
                        actor: active.actor,
                        action: active.definition.id.clone(),
                    });
                    self.complete_if_finished(context, update)?;
                }
                ActionMarkerKind::Effect(_) | ActionMarkerKind::Audio(_) => {}
            }
        }

        if !completed && active.elapsed_ticks < active.definition.timeline.duration_ticks {
            self.active = Some(active);
        } else if !completed {
            return Err(battle_error(
                "action reached its duration without completing",
                true,
            ));
        }
        Ok(())
    }

    fn resolve_hits(
        &mut self,
        active: &ActiveAction,
        context: &mut dyn GameServices,
        update: &mut CombatUpdate,
    ) -> Result<(), EngineError> {
        let caster = self
            .units
            .get(&active.actor)
            .cloned()
            .ok_or_else(|| battle_error("active caster disappeared", false))?;
        for target_id in &active.targets {
            let target = self
                .units
                .get(target_id)
                .cloned()
                .ok_or_else(|| battle_error("active target disappeared", false))?;
            if !target.alive {
                update.events.push(CombatEvent::DamageResolved {
                    source: caster.id,
                    target: target.id,
                    amount: 0,
                    hp_after: target.hp,
                    missed: true,
                });
                continue;
            }

            let hit_rate = (active.definition.hit_rate + caster.magic / 4 + caster.level
                - target.level)
                .clamp(1, 100);
            let missed = context.random_below(100) >= hit_rate as u32;
            let power = (active.definition.power - target.magic_defense).max(0);
            let damage = if missed {
                0
            } else {
                (caster.magic + caster.level).saturating_mul(power).max(0)
            };
            let hp_after = target.hp.saturating_sub(damage as u32);
            let defeated = target.alive && hp_after == 0;

            self.units.get_mut(target_id).unwrap().hp = hp_after;
            self.units.get_mut(target_id).unwrap().alive = hp_after > 0;
            update.events.push(CombatEvent::DamageResolved {
                source: caster.id,
                target: target.id,
                amount: damage,
                hp_after,
                missed,
            });
            if defeated {
                update
                    .events
                    .push(CombatEvent::UnitDefeated { unit: target.id });
            }
        }
        Ok(())
    }

    fn complete_if_finished(
        &mut self,
        context: &mut dyn GameServices,
        update: &mut CombatUpdate,
    ) -> Result<(), EngineError> {
        let players_alive = self
            .units
            .values()
            .any(|unit| unit.team == Team::Player && unit.alive);
        let enemies_alive = self
            .units
            .values()
            .any(|unit| unit.team == Team::Enemy && unit.alive);
        let outcome = if !players_alive {
            BattleOutcome::Defeat
        } else if !enemies_alive {
            BattleOutcome::Victory
        } else {
            return Ok(());
        };
        self.status = match outcome {
            BattleOutcome::Victory => BattleStatus::Victory,
            BattleOutcome::Defeat => BattleStatus::Defeat,
        };
        update.outcome = Some(outcome);
        update.events.push(CombatEvent::BattleCompleted { outcome });

        if outcome == BattleOutcome::Victory {
            let mut rewards = RewardBundle {
                resolution_id: self.next_resolution,
                ..RewardBundle::default()
            };
            for unit in self.units.values().filter(|unit| unit.team == Team::Enemy) {
                rewards.experience = rewards
                    .experience
                    .checked_add(unit.reward.experience)
                    .ok_or_else(|| battle_error("experience reward overflow", false))?;
                rewards.gil = rewards
                    .gil
                    .checked_add(unit.reward.gil)
                    .ok_or_else(|| battle_error("gil reward overflow", false))?;
                rewards.items.extend(unit.reward.items.clone());
            }
            context.commit_rewards(&rewards)?;
            self.next_resolution = self
                .next_resolution
                .checked_add(1)
                .ok_or_else(|| battle_error("reward resolution ID space exhausted", false))?;
            update.rewards = Some(rewards);
        }
        Ok(())
    }
}

fn validate_action(action: &ActionDefinition) -> Result<(), EngineError> {
    if action.timeline.duration_ticks == 0 {
        return Err(battle_error("action duration must be nonzero", true));
    }
    let mut previous = 0;
    let mut completions = 0;
    for marker in &action.timeline.markers {
        if marker.tick_offset == 0 || marker.tick_offset > action.timeline.duration_ticks {
            return Err(battle_error(
                "action marker is outside the action timeline",
                true,
            ));
        }
        if marker.tick_offset < previous {
            return Err(battle_error("action markers must be ordered", true));
        }
        if marker.kind == ActionMarkerKind::Complete {
            completions += 1;
            if marker.tick_offset != action.timeline.duration_ticks {
                return Err(battle_error(
                    "completion must occur at the action duration",
                    true,
                ));
            }
        }
        previous = marker.tick_offset;
    }
    if completions != 1 {
        return Err(battle_error(
            "action requires exactly one completion marker",
            true,
        ));
    }
    Ok(())
}

fn battle_error(context: impl Into<String>, recoverable: bool) -> EngineError {
    EngineError::new(
        ErrorCategory::Conflict,
        context,
        "tantalus-game/battle",
        recoverable,
    )
}
