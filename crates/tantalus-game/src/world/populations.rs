use super::*;
use std::collections::BTreeMap;
use tantalus_contracts::{ContentId, EngineError, EntityId, ErrorCategory, Point3, Tick};
use tantalus_core::Registry;

#[derive(Debug, Clone)]
struct RegionPopulation {
    definition: SpawnRegionDefinition,
    actor: ActorDefinition,
    behavior: BehaviorDefinition,
    actors: Vec<EntityId>,
    initialized: bool,
}

#[derive(Debug, Clone)]
pub struct WorldPopulation {
    regions: BTreeMap<ContentId, RegionPopulation>,
    actors: BTreeMap<EntityId, WorldActor>,
    next_entity: u64,
}

impl WorldPopulation {
    pub fn from_registry(
        registry: &Registry,
        region_ids: Vec<ContentId>,
    ) -> Result<Self, EngineError> {
        let mut regions = BTreeMap::new();
        for id in region_ids {
            let definition: SpawnRegionDefinition = registry_json(registry, &id)?;
            let actor: ActorDefinition = registry_json(registry, &definition.enemy)?;
            let behavior: BehaviorDefinition = registry_json(registry, &actor.behavior)?;
            regions.insert(
                id,
                RegionPopulation {
                    definition,
                    actor,
                    behavior,
                    actors: Vec::new(),
                    initialized: false,
                },
            );
        }
        Ok(Self {
            regions,
            actors: BTreeMap::new(),
            next_entity: 1,
        })
    }

    pub fn actor(&self, id: EntityId) -> Option<&WorldActor> {
        self.actors.get(&id)
    }

    pub fn actors(&self) -> impl Iterator<Item = &WorldActor> {
        self.actors.values()
    }

    pub fn update(&mut self, input: &WorldUpdate) -> Result<Vec<WorldCommand>, EngineError> {
        let mut commands = Vec::new();
        let mut regions = std::mem::take(&mut self.regions);
        for region in regions.values_mut() {
            let active = input.active_cells.contains(&region.definition.cell);
            if active && !region.initialized {
                for index in 0..region.definition.capacity {
                    let position = spawn_position(&region.definition, index);
                    let actor = self.spawn_actor(region, position, input.tick)?;
                    commands.push(WorldCommand::SpawnActor {
                        actor: actor.id,
                        position: actor.position,
                    });
                }
                region.initialized = true;
            }

            for actor_id in &region.actors {
                let Some(actor) = self.actors.get_mut(actor_id) else {
                    return Err(world_error(format!(
                        "region `{}` references missing actor {actor_id}",
                        region.definition.id
                    )));
                };

                if actor.lifecycle == ActorLifecycle::Defeated {
                    if actor.active {
                        commands.push(WorldCommand::DespawnActor { actor: actor.id });
                        actor.active = false;
                    }
                    if actor.respawn_at.is_some_and(|tick| input.tick >= tick) {
                        actor.lifecycle = ActorLifecycle::Alive;
                        actor.position = actor.home;
                        actor.behavior = ActorBehaviorState::Patrol;
                        actor.encounter_pending = false;
                        actor.respawn_at = None;
                        if active {
                            actor.active = true;
                            commands.push(WorldCommand::SpawnActor {
                                actor: actor.id,
                                position: actor.position,
                            });
                        }
                    }
                    continue;
                }
                if actor.lifecycle != ActorLifecycle::Alive {
                    continue;
                }

                if !active {
                    if actor.active {
                        commands.push(WorldCommand::DespawnActor { actor: actor.id });
                        actor.active = false;
                    }
                    continue;
                }
                if !actor.active {
                    actor.active = true;
                    commands.push(WorldCommand::SpawnActor {
                        actor: actor.id,
                        position: actor.position,
                    });
                }
                if actor.encounter_pending {
                    continue;
                }

                let perception = perceive(actor.position, &input.player, &region.behavior);
                let home_distance = squared_distance(actor.position, actor.home);
                let leash = u128::from(region.behavior.leash_range).pow(2);
                let previous_position = actor.position;
                let previous_behavior = actor.behavior;

                if home_distance > leash {
                    actor.behavior = ActorBehaviorState::ReturnHome;
                    actor.position =
                        move_toward(actor.position, actor.home, region.behavior.pursuit_speed);
                } else if perception.noticed_player {
                    match region.behavior.disposition {
                        Disposition::Aggressive => {
                            actor.behavior = ActorBehaviorState::Pursue;
                            actor.position = move_toward(
                                actor.position,
                                input.player.position,
                                region.behavior.pursuit_speed,
                            );
                            if perception.within_encounter_range {
                                actor.encounter_pending = true;
                                commands.push(WorldCommand::EncounterRequested { actor: actor.id });
                            }
                        }
                        Disposition::Fleeing => {
                            actor.behavior = ActorBehaviorState::Flee;
                            let target = flee_direction(actor.position, input.player.position);
                            actor.position =
                                move_toward(actor.position, target, region.behavior.pursuit_speed);
                        }
                        Disposition::Passive => {
                            actor.behavior = ActorBehaviorState::Idle;
                        }
                    }
                } else if actor.position != actor.home {
                    actor.behavior = ActorBehaviorState::ReturnHome;
                    actor.position =
                        move_toward(actor.position, actor.home, region.behavior.pursuit_speed);
                } else {
                    actor.behavior = ActorBehaviorState::Patrol;
                    let target = patrol_target(actor, &region.behavior);
                    actor.position =
                        move_toward(actor.position, target, region.behavior.pursuit_speed);
                    if actor.position == target {
                        actor.patrol_direction = -actor.patrol_direction;
                    }
                }

                if actor.behavior != previous_behavior {
                    commands.push(WorldCommand::SetBehavior {
                        actor: actor.id,
                        behavior: actor.behavior,
                    });
                }
                if actor.position != previous_position {
                    commands.push(WorldCommand::MoveActor {
                        actor: actor.id,
                        position: actor.position,
                    });
                }
            }
        }
        self.regions = regions;
        Ok(commands)
    }

    pub fn record_outcome(
        &mut self,
        actor: EntityId,
        outcome: EncounterOutcome,
        tick: Tick,
    ) -> Result<(), EngineError> {
        let Some(actor_state) = self.actors.get_mut(&actor) else {
            return Err(world_error(format!("unknown encounter actor {actor}")));
        };
        actor_state.encounter_pending = false;
        match outcome {
            EncounterOutcome::Defeated => {
                actor_state.lifecycle = ActorLifecycle::Defeated;
                let region = self.regions.get(&actor_state.region).ok_or_else(|| {
                    world_error(format!("actor {actor} references a missing spawn region"))
                })?;
                actor_state.respawn_at = match region.definition.respawn_ticks {
                    Some(delay) => Some(tick.checked_add(delay).ok_or_else(|| {
                        EngineError::new(
                            ErrorCategory::CapacityExhausted,
                            "respawn tick overflow",
                            "tantalus-game/world",
                            false,
                        )
                    })?),
                    None => None,
                };
            }
            EncounterOutcome::PlayerFled | EncounterOutcome::Cancelled => {}
        }
        Ok(())
    }

    fn spawn_actor(
        &mut self,
        region: &mut RegionPopulation,
        position: Point3,
        _tick: Tick,
    ) -> Result<&WorldActor, EngineError> {
        let id = EntityId::new(self.next_entity);
        self.next_entity = self.next_entity.checked_add(1).ok_or_else(|| {
            EngineError::new(
                ErrorCategory::CapacityExhausted,
                "world actor ID space exhausted",
                "tantalus-game/world",
                false,
            )
        })?;
        let actor = WorldActor {
            id,
            definition: region.actor.id.clone(),
            region: region.definition.id.clone(),
            position,
            home: position,
            lifecycle: ActorLifecycle::Alive,
            behavior: ActorBehaviorState::Patrol,
            active: true,
            encounter_pending: false,
            patrol_direction: 1,
            respawn_at: None,
        };
        self.actors.insert(id, actor);
        region.actors.push(id);
        self.actors
            .get(&id)
            .ok_or_else(|| world_error("new actor was not inserted"))
    }
}

fn registry_json<T>(registry: &Registry, id: &ContentId) -> Result<T, EngineError>
where
    T: serde::de::DeserializeOwned,
{
    let record = registry
        .get(id)
        .ok_or_else(|| world_error(format!("required world content `{id}` is missing")))?;
    serde_json::from_value(record.data.clone()).map_err(|error| {
        world_error(format!(
            "world content `{id}` has an invalid schema: {error}"
        ))
    })
}

fn spawn_position(region: &SpawnRegionDefinition, index: u16) -> Point3 {
    if index == 0 {
        return region.center;
    }
    let offset =
        i32::try_from(region.radius).unwrap_or(i32::MAX) / i32::from(region.capacity.max(1));
    let sign = if index.is_multiple_of(2) { -1 } else { 1 };
    Point3 {
        x: region.center.x + offset * i32::from(index) * sign,
        ..region.center
    }
}

fn patrol_target(actor: &WorldActor, behavior: &BehaviorDefinition) -> Point3 {
    Point3 {
        x: actor.home.x
            + i32::try_from(behavior.patrol_radius).unwrap_or(i32::MAX)
                * i32::from(actor.patrol_direction),
        ..actor.home
    }
}

fn squared_distance(left: Point3, right: Point3) -> u128 {
    let x = i128::from(left.x) - i128::from(right.x);
    let y = i128::from(left.y) - i128::from(right.y);
    let z = i128::from(left.z) - i128::from(right.z);
    (x * x + y * y + z * z) as u128
}

fn world_error(context: impl Into<String>) -> EngineError {
    EngineError::new(
        ErrorCategory::UnknownContent,
        context,
        "tantalus-game/world",
        true,
    )
}
