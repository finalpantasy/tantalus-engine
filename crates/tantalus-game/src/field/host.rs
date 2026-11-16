//! `LegacyEventHost` implementation over [`FieldWorld`].
//!
//! The host bridges decoded event programs and game state: `apply` folds host
//! events into the world, and every trait query answers from that world.
//! Reads the world cannot back (BGI navigation, custom variables, atan
//! tables) fail with explicit `UnknownContent` errors instead of inventing
//! values.

use super::world::{EncounterRequest, FieldWorld, Jump, Movement, Turn};
use tantalus_contracts::{EngineError, EntityId, ErrorCategory};
use tantalus_legacy::{
    CharacterStat, HostEvent, KeyState, LegacyEventHost, ObjectMemberRef, VariableMemory,
    VariableScope,
};

pub struct FieldHost<'a> {
    world: &'a mut FieldWorld,
}

impl<'a> FieldHost<'a> {
    pub fn new(world: &'a mut FieldWorld) -> Self {
        Self { world }
    }

    /// Reference CastFloatToIntWithChecking: round when unambiguous, else -1.
    fn cast_float_with_checking(value: f32) -> i32 {
        let floor = value.floor() as i32;
        let ceil = value.ceil() as i32;
        let round = value.round() as i32;
        if round == ceil {
            ceil
        } else if round == floor {
            floor
        } else {
            -1
        }
    }

    fn unknown(feature: &str) -> EngineError {
        EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field host cannot back `{feature}` yet"),
            "tantalus-game/field-host",
            false,
        )
    }

    /// Characters observed by BGI queries resolve through the UID map to an
    /// actor entity; positions then come from actor state.
    fn character_actor(&mut self, character: i32, context: &str) -> Result<EntityId, EngineError> {
        self.world
            .resolve_uid(character)
            .ok_or_else(|| Self::unknown(context))
    }

    fn actor_position(&self, actor: EntityId, context: &str) -> Result<[f32; 3], EngineError> {
        self.world
            .actors
            .get(&actor)
            .map(|state| state.position)
            .ok_or_else(|| {
                EngineError::new(
                    ErrorCategory::UnknownContent,
                    format!("{context}: unknown actor {actor}"),
                    "tantalus-game/field-host",
                    false,
                )
            })
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

impl FieldHost<'_> {
    /// `MJPOS` (EventEngine.DoEventCode.cs:2200): the controlled object's
    /// position projected onto the region's first edge, clamped to it.
    fn exit_position(&self, region: Option<EntityId>) -> (i32, i32) {
        let Some(position) = self
            .world
            .control_actor
            .and_then(|actor| self.world.actors.get(&actor))
            .map(|state| state.position)
        else {
            return (0, 0);
        };
        let Some(vertices) = region
            .and_then(|region| self.world.regions.get(&region))
            .map(|region| &region.vertices)
            .filter(|vertices| vertices.len() >= 2)
        else {
            return (0, 0);
        };
        let (x1, z1) = (i32::from(vertices[0][0]), i32::from(vertices[0][1]));
        let dx = i32::from(vertices[1][0]) - x1;
        let dz = i32::from(vertices[1][1]) - z1;
        let mut val = (dx * dx + dz * dz) >> 8;
        if val != 0 {
            let along = (dx as f32 * (position[0] - x1 as f32)) as i32
                + (dz as f32 * (position[2] - z1 as f32)) as i32;
            val = (along / val).clamp(0, 256);
        }
        (((val * dx) >> 8) + x1, ((val * dz) >> 8) + z1)
    }
}

impl LegacyEventHost for FieldHost<'_> {
    fn apply(&mut self, events: &[HostEvent]) -> Result<(), EngineError> {
        for event in events {
            // v1 member-target tracking: the reference resolves member
            // reads against the executing object; the closest observable
            // proxy at apply time is the actor an event operates on.
            match event {
                HostEvent::WalkActor { actor, .. }
                | HostEvent::PlayAnimation { actor, .. }
                | HostEvent::StopAnimation { actor }
                | HostEvent::StartActorTurn { actor, .. }
                | HostEvent::SetupJump { actor, .. }
                | HostEvent::ProgressJump { actor }
                | HostEvent::MoveActorToExit { actor }
                | HostEvent::SetActorPosition { actor, .. }
                | HostEvent::WalkTowardObject { actor, .. } => {
                    self.world.set_current_actor(Some(*actor));
                }
                _ => {}
            }
            match event {
                HostEvent::ShowWindow { window, text, .. } => {
                    self.world.windows.insert(*window, true);
                    self.world.window_texts.insert(*window, *text);
                    self.world.raise_text_signals(*text);
                }
                HostEvent::ShowObjectWindow { window, text, .. } => {
                    self.world.windows.insert(*window, true);
                    self.world.window_texts.insert(*window, *text);
                    self.world.raise_text_signals(*text);
                }
                HostEvent::SetDialogSignal { signal } => {
                    // SETSIGNAL (EventEngine.DoEventCode.cs:2927).
                    self.world.mes_signal = *signal;
                }
                HostEvent::CloseWindow { window } => {
                    self.world.windows.insert(*window, false);
                }
                HostEvent::DispatchCinematic {
                    operation,
                    argument,
                    auxiliary,
                } => {
                    // Frame counts come from the caller-provided movie
                    // inventory when available; without one a played movie
                    // finishes on its first frame, as if skipped.
                    let map = self.world.field_map_no;
                    let provider = self.world.fmv_frame_counts.clone();
                    self.world
                        .fmv
                        .dispatch(*operation, *argument, *auxiliary, map, {
                            let provider = provider;
                            move |disc, number| {
                                provider.as_ref().and_then(|provide| provide(disc, number))
                            }
                        });
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
                    self.world.delete_object(*target);
                }
                HostEvent::PlayAnimation { actor, .. } => {
                    let frames = self.world.animation_default_ticks;
                    let state = self.actor_mut(*actor, "play animation")?;
                    // `ExecAnim` (EventEngine.cs:1313) starts on `inFrame`;
                    // a ranged animation lasts its frame span.
                    state.animation_frame = u16::from(state.in_frame);
                    state.animation_frames = if state.out_frame == u8::MAX {
                        frames
                    } else {
                        u16::from(state.out_frame.abs_diff(state.in_frame)).max(1)
                    };
                }
                HostEvent::SetAnimationRange { actor, start, end } => {
                    let state = self.actor_mut(*actor, "set animation range")?;
                    state.in_frame = *start;
                    state.out_frame = *end;
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
                    // `StartTurn` (DoEventCode.cs:3356) clears the `AFRAME` range.
                    state.in_frame = 0;
                    state.out_frame = u8::MAX;
                    state.facing = facing_from_fixed_point(*angle);
                    state.turn = Some(Turn {
                        target: *angle,
                        speed: *speed,
                        remaining_ticks: ticks,
                    });
                }
                HostEvent::SetActorFacing { actor, angle } => {
                    let state = self.actor_mut(*actor, "set facing")?;
                    state.facing = facing_from_fixed_point(*angle);
                }
                HostEvent::SetObjectFlags { object, flags } => {
                    let state = self.actor_mut(*object, "set object flags")?;
                    state.flags = i32::from(*flags);
                }
                HostEvent::SetActorRadii {
                    actor, talk_radius, ..
                } => {
                    self.world.set_talk_radius(*actor, *talk_radius);
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
                    self.world.add_gil(*amount);
                }
                HostEvent::SetPartyHp { character, hp } => {
                    set_party_points(self.world, *character, *hp, 9999, true);
                }
                HostEvent::SetPartyMp { character, mp } => {
                    set_party_points(self.world, *character, *mp, 999, false);
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
                HostEvent::CalculateExitPosition { region } => {
                    let (x, z) = self.exit_position(*region);
                    self.world.set_system_variable(10, x);
                    self.world.set_system_variable(11, z);
                }
                HostEvent::InitializeObject { source, unique_id } => {
                    // `NewThread`: uid 0 defaults to the entry's own id; the
                    // object is the entry.
                    let uid = if *unique_id == 0 { *source } else { *unique_id };
                    let entity = EntityId::new(u64::from(*source));
                    self.world.map_uid(i32::from(uid), entity);
                    self.world.restore_object(entity);
                }
                HostEvent::DefineRegion { region, vertices } => {
                    self.world.regions.insert(
                        *region,
                        super::region::Region {
                            vertices: vertices.clone(),
                        },
                    );
                }
                HostEvent::DefinePlayerControl { actor } => {
                    self.world.control_actor = Some(*actor);
                    // The controlled actor is the interactive player for
                    // talk triggers, region occupancy, and walk input.
                    self.world.set_player(*actor);
                }
                HostEvent::SetUserControl { enabled } => {
                    self.world.user_control = *enabled;
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
        // `GetUIDFromLocalUID`: 250 is the controlled object. The VM has
        // already replaced 255 (the running object) with its entity.
        let mut reference = reference;
        if reference.object == EntityId::new(CONTROL_OBJECT_UID)
            && let Some(control) = self.world.control_actor.or(self.world.player())
        {
            reference.object = control;
        }
        if let Some(value) = self
            .world
            .object_member(&(reference.object, reference.member))
        {
            return Ok(value);
        }
        // Facing-gate detection: scenes polling the player actor's facing
        // (member 3) are waiting for the player to face a direction.
        if reference.member == 3 && self.world.player() == Some(reference.object) {
            self.world.player_facing_polls += 1;
        }
        // Referenced objects always exist in the reference's object table;
        // implicitly register a zero-state actor for uids no event declared
        // yet (dynamic NEW3 objects observed before their init runs).
        if !self.world.actors.contains_key(&reference.object) {
            self.world.add_actor(reference.object, [0.0; 3]);
        }
        let state = self
            .world
            .actors
            .get(&reference.object)
            .ok_or_else(|| Self::unknown("object member read for an unknown actor"))?;
        // Reference getvobj members (EBin.cs): 0 pos.x, 1 -pos.y, 2 pos.z
        // (floats converted with the floor/ceil/round checking rule), 3
        // facing fixed-point, 4 flags, 5 uid, 6 level, 7 anim frame.
        let position = match reference.member {
            0 => Some(state.position[0]),
            1 => Some(-state.position[1]),
            2 => Some(state.position[2]),
            _ => None,
        };
        if let Some(value) = position {
            return Ok(Self::cast_float_with_checking(value));
        }
        // Members 3/4/6/7: facing, flags, level, animation frame, tracked
        // in actor state (facing stores the raw angle operand).
        match reference.member {
            3 => Ok(i32::from(state.facing)),
            4 => Ok(state.flags),
            5 => self
                .world
                .uid_of(reference.object)
                .ok_or_else(|| Self::unknown("object member uid without a UID mapping")),
            6 => Ok(state.level),
            7 => Ok(i32::from(state.animation_frame)),
            // Members 8 and above fall through to the reference's
            // battle-data lookup, which yields zero for non-battle objects.
            8.. => Ok(0),
            _ => Err(Self::unknown("object member read is not backed yet")),
        }
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
        // 14 GetCinematicFrame and 15 SyncCinematic come from the movie
        // service (`GetSysvar.cs`, `fldfmv.FF9FieldFMVDispatch` 4 and 3).
        match variable {
            // 0 GetRandom: `Comn.random8()`, 0..=255.
            0 => return Ok(i32::from(self.world.next_random_u8())),
            // 2 IsMovementEnabled: `EventContext.usercontrol`.
            2 => return Ok(i32::from(self.world.user_control)),
            // 8 GetDialogProgression: `ETb.gMesSignal`.
            8 => return Ok(self.world.mes_signal),
            // 9 GetDialogChoice: `ETb.GetChoose()`.
            9 => return Ok(self.world.dialog_choice),
            14 => return Ok(self.world.fmv.frame),
            15 => return Ok(self.world.fmv.sync()),
            _ => {}
        }
        // Engine state is zero-initialized in the reference, so unset
        // system variables read as zero rather than erroring.
        Ok(self.world.system_variable(variable).unwrap_or(0))
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

    fn read_atan_entry(&mut self, offset: i32) -> Result<u16, EngineError> {
        if offset < 0 {
            return Err(Self::unknown("negative atan table offset"));
        }
        self.world
            .atan_entry(offset as usize)
            .ok_or_else(|| Self::unknown("atan table offset outside the provided table"))
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

    fn read_bgi_character_triangle(&mut self, character: i32) -> Result<i32, EngineError> {
        let actor = self.character_actor(character, "BGI character triangle")?;
        let position = self.actor_position(actor, "BGI character triangle")?;
        let triangle = self
            .world
            .navigation_triangle_at(position)
            .ok_or_else(|| Self::unknown("BGI character triangle projection"))?;
        i32::try_from(triangle).map_err(|_| Self::unknown("BGI triangle index exceeds i32"))
    }

    fn read_bgi_character_floor(&mut self, character: i32) -> Result<i32, EngineError> {
        let actor = self.character_actor(character, "BGI character floor")?;
        let _ = self.actor_position(actor, "BGI character floor")?;
        // The reference reports the character's walkmesh floor; without
        // per-actor floor tracking the active floor is the honest answer.
        let floor = self
            .world
            .navigation()
            .map(|mesh| i32::from(mesh.active_floor))
            .ok_or_else(|| Self::unknown("BGI character floor without a navigation mesh"))?;
        Ok(floor)
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
        // Characters without registered stats read as zero, matching the
        // zero-initialized engine state convention.
        let stats = self
            .world
            .character_stats
            .get(&character)
            .copied()
            .unwrap_or_default();
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

    fn object_uid(&mut self, entity: EntityId) -> Option<i32> {
        self.world.uid_of(entity)
    }

    fn control_object(&mut self) -> Option<EntityId> {
        self.world.control_actor.or(self.world.player())
    }

    fn object_deleted(&mut self, entity: EntityId) -> bool {
        self.world.is_object_deleted(entity)
    }

    fn read_custom_variable(&mut self, _variable: u16) -> Result<i32, EngineError> {
        Err(Self::unknown("custom variable"))
    }

    fn variable_memory(&mut self, scope: VariableScope) -> Option<&mut VariableMemory> {
        match scope {
            VariableScope::Global => Some(&mut self.world.global_memory),
            VariableScope::Map => Some(&mut self.world.map_memory),
            VariableScope::Instance => None,
        }
    }
}

/// Local object uid that names the controlled object (`EventContext.controlUID`).
const CONTROL_OBJECT_UID: u64 = 250;

/// Facing as `getvobj` member 3 reports it: the VM passes `DIRE`/`TURNA`
/// angles as `angle << 4` (the reference's 12-bit fixed point), and the
/// read returns `fixed >> 4 & 255` (0 south, 64 west, 128 north, 192 east).
fn facing_from_fixed_point(angle: i16) -> i16 {
    (angle >> 4) & 0xFF
}

/// `ff9play.CharacterOldIndexToID`: scripts name characters by the old
/// index (Beatrix 8, Cinna 9, Marcus 10, Blank 11); the player records are
/// keyed by `CharacterId` (Cinna 8, Marcus 9, Blank 10, Beatrix 11).
fn character_old_index_to_id(old: i32) -> i32 {
    match old {
        8 => 11,
        9 => 8,
        10 => 9,
        11 => 10,
        other => other,
    }
}

/// Field `SETHP` (0xF1) and `SETMP` (0xF2) from `EventEngine.DoEventCode`:
/// the new value is clamped to the maximum, and `full` (9999 HP, 999 MP)
/// restores fully. A field heal named on Beatrix also heals Cinna, Marcus
/// and Blank by the same share of their maximum (tents, inns, water).
fn set_party_points(
    world: &mut super::world::FieldWorld,
    old: i32,
    value: i32,
    full: i32,
    hp: bool,
) {
    const BEATRIX: i32 = 11;
    const AMARANT: i32 = 7;
    let character = character_old_index_to_id(old);
    let Some(record) = world.character_stats.get_mut(&character) else {
        return;
    };
    let (current, max) = if hp {
        (record.current_hp, record.max_hp)
    } else {
        (record.current_mp, record.max_mp)
    };
    let share = if value <= current {
        -1.0
    } else if value == full {
        1.0
    } else if max > 0 {
        (value - current) as f32 / max as f32
    } else {
        0.0
    };
    let value = if value == full { i32::MAX } else { value };
    let set = value.min(max).max(0);
    if hp {
        record.current_hp = set;
    } else {
        record.current_mp = set;
    }
    if character == BEATRIX && share >= 0.0 {
        for (&id, other) in world.character_stats.iter_mut() {
            if id > AMARANT && id != BEATRIX {
                let (current, max) = if hp {
                    (&mut other.current_hp, other.max_hp)
                } else {
                    (&mut other.current_mp, other.max_mp)
                };
                *current = ((*current as f32 + share * max as f32) as i32).min(max);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::world::CharacterStats;

    fn stats(hp: i32, max_hp: i32) -> CharacterStats {
        CharacterStats {
            current_hp: hp,
            max_hp,
            current_mp: 0,
            max_mp: 20,
        }
    }

    #[test]
    fn set_party_points_maps_old_index_clamps_and_spreads_beatrix_heals() {
        let mut world = FieldWorld::new();
        for id in [0, 8, 9, 10, 11] {
            world.character_stats.insert(id, stats(10, 100));
        }
        // Old index 11 is Blank (id 10); 9999 is a full heal.
        set_party_points(&mut world, 11, 9999, 9999, true);
        assert_eq!(world.character_stats[&10].current_hp, 100);
        // Values above the maximum clamp.
        set_party_points(&mut world, 0, 500, 9999, true);
        assert_eq!(world.character_stats[&0].current_hp, 100);
        // Old index 8 is Beatrix (id 11): a field heal of half her maximum
        // also gives Cinna and Marcus half of theirs.
        set_party_points(&mut world, 8, 60, 9999, true);
        assert_eq!(world.character_stats[&11].current_hp, 60);
        assert_eq!(world.character_stats[&8].current_hp, 60);
        assert_eq!(world.character_stats[&9].current_hp, 60);
        assert_eq!(world.character_stats[&10].current_hp, 100);
        // 999 MP is a full restore.
        set_party_points(&mut world, 0, 999, 999, false);
        assert_eq!(world.character_stats[&0].current_mp, 20);
    }
}
