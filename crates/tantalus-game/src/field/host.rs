//! `LegacyEventHost` implementation over [`FieldWorld`].
//!
//! The host bridges decoded event programs and game state: `apply` folds host
//! events into the world, and every trait query answers from that world.
//! Reads the world cannot back (BGI navigation, custom variables, atan
//! tables) fail with explicit `UnknownContent` errors instead of inventing
//! values.

use super::world::{EncounterRequest, FieldWorld, Jump, Movement, Turn};
use tantalus_contracts::{EngineError, EntityId, ErrorCategory};
use tantalus_legacy::{CharacterStat, HostEvent, KeyState, LegacyEventHost, ObjectMemberRef};

const MAX_GIL: u32 = 9_999_999;

pub struct FieldHost<'a> {
    world: &'a mut FieldWorld,
}

impl<'a> FieldHost<'a> {
    pub fn new(world: &'a mut FieldWorld) -> Self {
        Self { world }
    }

    fn unknown(feature: &str) -> EngineError {
        EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field host cannot back `{feature}` yet"),
            "tantalus-game/field-host",
            false,
        )
    }

    fn actor_mut(
        &mut self,
        actor: EntityId,
        context: &str,
    ) -> Result<&mut super::world::ActorState, EngineError> {
        self.world.actors.get_mut(&actor).ok_or_else(|| {
            EngineError::new(
                ErrorCategory::UnknownContent,
                format!("{context}: unknown actor {actor}"),
                "tantalus-game/field-host",
                false,
            )
        })
    }
}

impl LegacyEventHost for FieldHost<'_> {
    fn apply(&mut self, events: &[HostEvent]) -> Result<(), EngineError> {
        for event in events {
            match event {
                HostEvent::ShowWindow { window, .. }
                | HostEvent::ShowObjectWindow { window, .. } => {
                    self.world.windows.insert(*window, true);
                }
                HostEvent::CloseWindow { window } => {
                    self.world.windows.insert(*window, false);
                }
                HostEvent::CloseAllWindows => {
                    for open in self.world.windows.values_mut() {
                        *open = false;
                    }
                }
                HostEvent::WalkActor { actor, destination } => {
                    let state = self.actor_mut(*actor, "walk")?;
                    state.movement = Some(Movement {
                        destination: [destination[0] as f32, destination[1] as f32],
                    });
                    state.exit_walk = false;
                }
                HostEvent::WalkTowardObject { actor, target } => {
                    // Re-aimed on every re-emitted event so following tracks
                    // a moving target; arrival clears the movement.
                    let target_position = self
                        .world
                        .actors
                        .get(target)
                        .map(|state| state.position)
                        .ok_or_else(|| {
                            EngineError::new(
                                ErrorCategory::UnknownContent,
                                format!("walk toward: unknown target {target}"),
                                "tantalus-game/field-host",
                                false,
                            )
                        })?;
                    let state = self.actor_mut(*actor, "walk toward")?;
                    if state.movement.is_none() {
                        state.movement = Some(Movement {
                            destination: [target_position[0], target_position[2]],
                        });
                    } else if let Some(movement) = &mut state.movement {
                        movement.destination = [target_position[0], target_position[2]];
                    }
                }
                HostEvent::MoveActor { actor, x, z } => {
                    let state = self.actor_mut(*actor, "move")?;
                    state.position[0] = *x as f32;
                    state.position[2] = *z as f32;
                    state.movement = None;
                }
                HostEvent::MoveActorToExit { actor } => {
                    let state = self.actor_mut(*actor, "exit walk")?;
                    state.exit_walk = true;
                    state.movement = None;
                }
                HostEvent::SetActorPosition { actor, position } => {
                    let state = self.actor_mut(*actor, "set position")?;
                    state.position = [position[0] as f32, position[1] as f32, position[2] as f32];
                    state.movement = None;
                }
                HostEvent::DeleteObject { target } => {
                    self.world.actors.remove(target);
                }
                HostEvent::PlayAnimation { actor, .. } => {
                    let frames = self.world.animation_default_ticks;
                    let state = self.actor_mut(*actor, "play animation")?;
                    state.animation_frames = frames;
                }
                HostEvent::StopAnimation { actor } => {
                    let state = self.actor_mut(*actor, "stop animation")?;
                    state.animation_frames = 0;
                }
                HostEvent::StartActorTurn {
                    actor,
                    angle,
                    speed,
                } => {
                    let ticks = 1 + ((*speed).max(1) as u32);
                    let state = self.actor_mut(*actor, "turn")?;
                    state.turn = Some(Turn {
                        target: *angle,
                        speed: *speed,
                        remaining_ticks: ticks,
                    });
                }
                HostEvent::SetupJump {
                    actor,
                    x,
                    z,
                    y,
                    steps,
                } => {
                    let state = self.actor_mut(*actor, "setup jump")?;
                    let from = state.position;
                    state.jump = Some(Jump {
                        from,
                        to: [*x as f32, *y as f32, *z as f32],
                        steps: (*steps).max(1),
                        frame: 0,
                    });
                }
                HostEvent::ProgressJump { actor } => {
                    let state = self.actor_mut(*actor, "progress jump")?;
                    if let Some(jump) = &mut state.jump {
                        jump.frame = jump.frame.saturating_add(1);
                        let steps = i32::from(jump.steps);
                        let frame = i32::from(jump.frame).min(steps);
                        let from = jump.from;
                        let to = jump.to;
                        // Reference JUMP3 interpolation, including the arc term.
                        state.position = [
                            (from[0] * (steps - frame) as f32 + to[0] * frame as f32)
                                / steps as f32,
                            from[1] - (frame * (frame << 3)) as f32
                                + frame as f32 * (to[1] - from[1]) / steps as f32
                                + (frame * (steps << 3)) as f32,
                            (from[2] * (steps - frame) as f32 + to[2] * frame as f32)
                                / steps as f32,
                        ];
                        if jump.frame >= jump.steps {
                            state.jump = None;
                        }
                    }
                }
                HostEvent::AddItem { item, count } => {
                    let entry = self.world.items.entry(*item).or_insert(0);
                    *entry = entry.saturating_add(*count).min(99);
                }
                HostEvent::RemoveItem { item, count } => {
                    let entry = self.world.items.entry(*item).or_insert(0);
                    *entry = (*entry - *count).max(0);
                }
                HostEvent::AddGil { amount } => {
                    self.world.gil = self.world.gil.saturating_add(*amount).min(MAX_GIL);
                }
                HostEvent::RemoveGil { amount } => {
                    self.world.gil = self.world.gil.saturating_sub(*amount);
                }
                HostEvent::SetFieldName { text } => {
                    self.world.field_name = Some(*text);
                }
                HostEvent::ResetFieldName => {
                    self.world.field_name = None;
                }
                HostEvent::SetTimerTime { seconds } => {
                    self.world.timer.seconds = *seconds;
                }
                HostEvent::SetTimerRunning { running } => {
                    self.world.timer.running = *running;
                }
                HostEvent::ShowTimer { show } => {
                    self.world.timer.displayed = *show;
                }
                HostEvent::OpenPartyMenu { .. } => {
                    self.world.open_party_menu();
                }
                HostEvent::StartBattle {
                    rush,
                    group,
                    battle,
                    steiner_state,
                } => {
                    self.world.encounters.push(EncounterRequest {
                        rush: *rush,
                        group: *group,
                        battle: *battle,
                        steiner_state: *steiner_state,
                    });
                }
                HostEvent::ChangeField { field } => {
                    self.world.field_transitions.push(*field);
                }
                HostEvent::ChangeWorldMap { destination } => {
                    self.world.push_world_transition(*destination);
                }
                HostEvent::RequestScriptAsync { target, .. }
                | HostEvent::RequestScriptSync { target, .. } => {
                    self.world.pend_request(*target);
                }
                HostEvent::RequestCallerAsync { level, .. }
                | HostEvent::RequestCallerSync { level, .. } => {
                    self.world.pend_caller(*level);
                }
                HostEvent::RunSharedScript { actor, .. } => {
                    self.world.start_shared_script(*actor);
                }
                HostEvent::InitializeObject { unique_id, .. } => {
                    self.world
                        .map_uid(i32::from(*unique_id), EntityId::new(u64::from(*unique_id)));
                }
                // Presentation-only or engine-owned effects this world does
                // not observe back.
                _ => {}
            }
        }
        Ok(())
    }

    fn window_is_open(&self, window: u8) -> bool {
        self.world.windows.get(&window).copied().unwrap_or(false)
    }

    fn read_object_member(&mut self, reference: ObjectMemberRef) -> Result<i32, EngineError> {
        if let Some(value) = self
            .world
            .object_member(&(reference.object, reference.member))
        {
            return Ok(value);
        }
        Err(Self::unknown("object member read"))
    }

    fn select_object_member_target(&mut self, slot: Option<u8>) -> Result<EntityId, EngineError> {
        match slot {
            Some(index) => {
                let entity = EntityId::new(u64::from(index));
                self.world.set_member_target(Some(entity));
                Ok(entity)
            }
            None => self
                .world
                .member_target()
                .ok_or_else(|| Self::unknown("member target without selection")),
        }
    }

    fn write_object_member(
        &mut self,
        reference: ObjectMemberRef,
        value: i32,
    ) -> Result<(), EngineError> {
        self.world
            .set_object_member((reference.object, reference.member), value);
        Ok(())
    }

    fn read_system_variable(&mut self, variable: u8) -> Result<i32, EngineError> {
        self.world
            .system_variable(variable)
            .ok_or_else(|| Self::unknown("system variable"))
    }

    fn read_system_list(&mut self, index: u8) -> Result<i32, EngineError> {
        self.world
            .system_list(index)
            .ok_or_else(|| Self::unknown("system list"))
    }

    fn write_system_list(&mut self, index: u8, value: i32) -> Result<(), EngineError> {
        self.world.set_system_list(index, value);
        Ok(())
    }

    fn read_atan_entry(&mut self, _offset: i32) -> Result<u16, EngineError> {
        Err(Self::unknown("atan table"))
    }

    fn resolve_object_uid(&mut self, uid: i32) -> Result<EntityId, EngineError> {
        self.world
            .resolve_uid(uid)
            .ok_or_else(|| Self::unknown("object uid resolution"))
    }

    fn find_object_uid(&mut self, uid: u8) -> Result<Option<i32>, EngineError> {
        Ok(self
            .world
            .resolve_uid(i32::from(uid))
            .map(|entity| i32::try_from(entity.get()).unwrap_or(i32::MAX)))
    }

    fn read_object_position(&mut self, object: EntityId) -> Result<[f32; 3], EngineError> {
        self.world
            .actors
            .get(&object)
            .map(|state| state.position)
            .ok_or_else(|| Self::unknown("object position"))
    }

    fn read_item_count(&mut self, item: i32) -> Result<i32, EngineError> {
        Ok(self.world.items.get(&item).copied().unwrap_or(0))
    }

    fn read_animation_frame(
        &mut self,
        actor: EntityId,
        _animation: i32,
    ) -> Result<i32, EngineError> {
        self.world
            .actors
            .get(&actor)
            .map(|state| i32::from(state.animation_frames))
            .ok_or_else(|| Self::unknown("animation frame"))
    }

    fn read_bgi_animation_frame(&mut self, _animation: u32) -> Result<i32, EngineError> {
        Err(Self::unknown("BGI animation frame"))
    }

    fn read_bgi_character_triangle(&mut self, _character: i32) -> Result<i32, EngineError> {
        Err(Self::unknown("BGI character triangle"))
    }

    fn read_bgi_character_floor(&mut self, _character: i32) -> Result<i32, EngineError> {
        Err(Self::unknown("BGI character floor"))
    }

    fn read_character_max_mp(&mut self, character: i32) -> Result<i32, EngineError> {
        self.world
            .character_stats
            .get(&character)
            .map(|stats| stats.max_mp)
            .ok_or_else(|| Self::unknown("character max MP"))
    }

    fn party_contains(&mut self, character: i32) -> Result<bool, EngineError> {
        Ok(self.world.party.contains(&character))
    }

    fn add_party_member(&mut self, character: i32) -> Result<bool, EngineError> {
        // Reference failure flag: false on success.
        if self.world.party.len() >= 4 || self.world.party.contains(&character) {
            return Ok(true);
        }
        self.world.party.push(character);
        Ok(false)
    }

    fn party_menu_is_open(&self) -> bool {
        self.world.party_menu_open()
    }

    fn actor_turn_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError> {
        Ok(self
            .world
            .actors
            .get(&actor)
            .map(|state| state.turn.is_some())
            .unwrap_or(false))
    }

    fn shared_script_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError> {
        Ok(self.world.shared_script_active(actor))
    }

    fn actor_animation_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError> {
        Ok(self
            .world
            .actors
            .get(&actor)
            .map(|state| state.animation_frames > 0)
            .unwrap_or(false))
    }

    fn field_actor_movement_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError> {
        Ok(self
            .world
            .actors
            .get(&actor)
            .map(|state| state.movement.is_some())
            .unwrap_or(false))
    }

    fn field_exit_movement_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError> {
        Ok(self
            .world
            .actors
            .get(&actor)
            .map(|state| state.exit_walk && !self.world.exit_arrivals.contains(&actor))
            .unwrap_or(false))
    }

    fn actor_jump_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError> {
        Ok(self
            .world
            .actors
            .get(&actor)
            .map(|state| state.jump.is_some())
            .unwrap_or(false))
    }

    fn read_random_u8(&mut self) -> Result<u8, EngineError> {
        Ok(self.world.next_random_u8())
    }

    fn read_key_on(&mut self) -> Result<i32, EngineError> {
        Ok(self.world.input_mask())
    }

    fn read_key_state(&mut self, state: KeyState) -> Result<i32, EngineError> {
        match state {
            KeyState::Off => Ok(i32::from(self.world.input_mask() == 0)),
            KeyState::Held => Ok(self.world.input_mask()),
        }
    }

    fn read_character_stat(
        &mut self,
        character: i32,
        stat: CharacterStat,
    ) -> Result<i32, EngineError> {
        let stats = self
            .world
            .character_stats
            .get(&character)
            .ok_or_else(|| Self::unknown("character stat"))?;
        Ok(match stat {
            CharacterStat::CurrentHp => stats.current_hp,
            CharacterStat::MaxHp => stats.max_hp,
            CharacterStat::CurrentMp => stats.current_mp,
        })
    }

    fn caller_request_acceptable(&mut self, level: i32) -> Result<bool, EngineError> {
        Ok(!self.world.caller_pending(level))
    }

    fn script_request_acceptable(
        &mut self,
        target: EntityId,
        _level: i32,
    ) -> Result<bool, EngineError> {
        Ok(!self.world.request_pending(target))
    }

    fn script_request_is_complete(&mut self, entity: EntityId) -> Result<bool, EngineError> {
        Ok(!self.world.request_pending(entity))
    }

    fn read_custom_variable(&mut self, _variable: u16) -> Result<i32, EngineError> {
        Err(Self::unknown("custom variable"))
    }
}
