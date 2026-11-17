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

impl FieldHost<'_> {
    /// Keeps the model, animation and visibility state the renderer needs.
    /// Events for an actor that does not exist are ignored here; the
    /// handlers below report them where the reference does.
    /// Keeps the camera scripts the renderer's view needs.
    fn record_camera(&mut self, event: &HostEvent) {
        use super::world::CameraRequest;
        let request = match *event {
            HostEvent::MoveCamera {
                x,
                y,
                duration,
                mode,
            } => CameraRequest::Move {
                x,
                y,
                frames: duration,
                mode,
            },
            HostEvent::ReleaseCamera { duration, mode } => CameraRequest::Release {
                frames: duration,
                mode,
            },
            HostEvent::EnableCameraServices {
                active,
                frame_count,
                mode,
            } => CameraRequest::Services {
                active: active != 0,
                frames: frame_count,
                mode,
            },
            HostEvent::SetCameraFollowHeight { height } => CameraRequest::FollowHeight(height),
            HostEvent::LockCameraFollow => CameraRequest::FollowLock(true),
            HostEvent::UnlockCameraFollow => CameraRequest::FollowLock(false),
            HostEvent::SetCameraBounds { camera, bounds } => {
                CameraRequest::Bounds { camera, bounds }
            }
            _ => return,
        };
        let player = self.world.view_player();
        self.world.view.apply(request, player);
        // Scripts tween the aim height every frame; keep only changes.
        if self.world.camera_log.last() == Some(&request) {
            return;
        }
        self.world.camera_log.push(request);
    }

    /// Keeps the light and shadow scripts for the renderer.
    fn record_light(&mut self, event: &HostEvent) {
        use super::world::{
            ActorShadowScript, CHAR_ATTR_SHADOW_LOCK, CHAR_ATTR_SHADOW_OFF, LightChange,
        };
        fn entry(world: &mut super::world::FieldWorld, actor: EntityId) -> &mut ActorShadowScript {
            world.actor_shadows.entry(actor).or_default()
        }
        match *event {
            HostEvent::SetShadowEnabled { actor, enabled } => {
                let script = entry(self.world, actor);
                if enabled {
                    script.attr &= !CHAR_ATTR_SHADOW_OFF;
                } else {
                    script.attr |= CHAR_ATTR_SHADOW_OFF;
                }
            }
            HostEvent::SetShadowSize { actor, x, z } => {
                entry(self.world, actor).scale = Some([x, z])
            }
            HostEvent::SetShadowOffset { actor, x, z } => {
                entry(self.world, actor).offset = Some([i32::from(x), i32::from(z)]);
            }
            HostEvent::LockShadowRotation { actor, rotation } => {
                let script = entry(self.world, actor);
                script.attr |= CHAR_ATTR_SHADOW_LOCK;
                script.rotation = Some(rotation);
            }
            HostEvent::UnlockShadowRotation { actor } => {
                entry(self.world, actor).attr &= !CHAR_ATTR_SHADOW_LOCK;
            }
            HostEvent::SetShadowAmplifier { actor, factor } => {
                entry(self.world, actor).amp = Some(factor)
            }
            HostEvent::SetCharacterAttribute { actor, set, flags } => {
                let script = entry(self.world, actor);
                if set {
                    script.attr |= flags as u32;
                } else {
                    script.attr &= !(flags as u32);
                }
            }
            HostEvent::SetModelColor {
                object,
                actor,
                red,
                green,
                blue,
            } => {
                let target = if object == 255 {
                    actor
                } else {
                    self.world.resolve_uid(object)
                };
                if let Some(actor) = target {
                    self.world.actor_shadows.entry(actor).or_default().color =
                        Some([red, green, blue]);
                }
            }
            HostEvent::SetDefaultLight {
                kind,
                color,
                shadow_intensity,
                shadow_scale,
            } => self.world.light_changes.push(LightChange::Default {
                kind,
                color,
                shadow_intensity,
                shadow_scale,
            }),
            HostEvent::SetFloorLight {
                floor,
                color,
                shadow_intensity,
                shadow_scale,
            } => self.world.light_changes.push(LightChange::Floor {
                floor,
                color,
                shadow_intensity,
                shadow_scale,
            }),
            HostEvent::SetObjectScale {
                actor,
                scale,
                vertical_scale,
            } => {
                let look = self.world.actor_presentation.entry(actor).or_default();
                look.scale = scale;
                look.vertical_scale = vertical_scale;
            }
            HostEvent::SetHeadFocusMasks {
                actor,
                self_mask,
                target_mask,
            } => {
                let look = self.world.actor_presentation.entry(actor).or_default();
                look.head_focus_self = self_mask;
                look.head_focus_target = target_mask;
            }
            HostEvent::SetHeadFocusFlags { actor, flags } => {
                self.world
                    .actor_presentation
                    .entry(actor)
                    .or_default()
                    .head_focus_flags = flags;
            }
            HostEvent::ShowBubbleIcon { icon } => self.world.bubble_icon = Some(icon),
            HostEvent::SetHereIcon { mode } => {
                self.world.here_icon = mode;
                // `EIcon.SetHereIcon`: no importance cancels the icon; above 2 it always
                // starts the 60 tick timer, otherwise only while the player has control.
                if mode <= 0 {
                    self.world.here_icon_timer = 0;
                } else if mode > 2 || (self.world.user_control && mode == 2) {
                    self.world.here_icon_timer = 60;
                }
            }
            HostEvent::SetActiveTimeEventIcon { mode } => self.world.ate_icon_mode = mode,
            HostEvent::SetTileShadeColor { overlay, color } => {
                self.world.tile_shade.insert(overlay, color);
            }
            HostEvent::ApplyFadeFilter {
                mode,
                frame,
                cyan,
                magenta,
                yellow,
            } => {
                let slot = usize::from(mode & 2 != 0);
                self.world.fade_filter = Some(super::world::FadeFilter {
                    mode,
                    frames: frame,
                    color: [cyan, magenta, yellow],
                    elapsed: 0,
                    from: self.world.fade_current[slot],
                });
            }
            HostEvent::SetModelLight {
                model,
                color,
                shadow_intensity,
                shadow_scale,
            } => self.world.light_changes.push(LightChange::Model {
                model,
                color,
                shadow_intensity,
                shadow_scale,
            }),
            _ => {}
        }
    }

    /// Keeps the layer scroll scripts for the renderer's `FieldAnimation`.
    fn record_overlay_motion(&mut self, event: &HostEvent) {
        use super::world::OverlayMotion as Motion;
        let motion = match *event {
            HostEvent::SetTileLoop {
                block,
                enabled,
                x_speed,
                y_speed,
            } => Motion::Loop {
                block,
                enabled: enabled != 0,
                speed: [x_speed, y_speed],
            },
            HostEvent::SetTileParallax {
                block,
                enabled,
                x_speed,
                y_speed,
            } => Motion::Parallax {
                block,
                enabled: enabled != 0,
                speed: [x_speed, y_speed],
            },
            HostEvent::SetTileOrigin { block, x, y } => Motion::Origin { block, x, y },
            HostEvent::MoveTile { overlay, offset } => Motion::Move {
                block: overlay,
                offset,
            },
            HostEvent::SetTileScrollWithOffset {
                overlay,
                enabled,
                speed,
                offset,
                is_x_offset,
            } => Motion::ScrollWithOffset {
                block: overlay,
                enabled: enabled != 0,
                speed,
                offset,
                is_x_offset: is_x_offset != 0,
            },
            HostEvent::SetTileLoopType {
                overlay,
                screen_anchored,
            } => Motion::LoopType {
                block: overlay,
                screen_anchored,
            },
            HostEvent::DefineTileViewport {
                viewport,
                x,
                y,
                width,
                height,
            } => Motion::Viewport {
                viewport,
                position: [x, y],
                size: [width, height],
            },
            _ => return,
        };
        self.world.overlay_motion.push(motion);
    }

    /// Keeps the background tile-block changes the renderer needs.
    fn record_overlay(&mut self, event: &HostEvent) {
        let (block, change): (i32, &dyn Fn(&mut super::world::OverlayChange)) = match event {
            HostEvent::SetTileBlockActive { block, active } => {
                (*block, &|overlay| overlay.active = Some(*active))
            }
            HostEvent::SetTileOrigin { block, x, y } => {
                (*block, &|overlay| overlay.origin = Some([*x, *y]))
            }
            HostEvent::AttachTile {
                overlay,
                x,
                y,
                filter_mode,
                color,
            } => (*overlay, &|tile| {
                tile.attached = Some([*x, *y]);
                if *filter_mode >= 0 {
                    tile.border = Some(*color);
                }
            }),
            _ => return,
        };
        if let Ok(block) = u16::try_from(block) {
            change(self.world.overlays.entry(block).or_default());
        }
    }

    /// The `BGA*` opcodes and `SETCAM` (`EBG_anim*`); only the service
    /// in `tile_animation` turns them into overlay visibility.
    fn record_tile_animation(&mut self, event: &HostEvent) {
        use super::tile_animation as anim;
        let world = &mut *self.world;
        if world.tile_animations.is_empty() {
            return;
        }
        let animations = &mut world.tile_animations;
        match event {
            HostEvent::RunTileAnimation {
                animation,
                start_frame,
            } => {
                // Crystal World restores a missing animation (Memoria).
                if world.field_map_no == 2925 && *animation == 0 {
                    anim::animate(animations, 1, 0);
                    anim::add_flags(animations, 1, i32::from(anim::FLAG_LOOP));
                    anim::set_rate(animations, 1, 128);
                }
                anim::animate(animations, *animation, *start_frame);
            }
            HostEvent::SetTileAnimationActive { animation, active } => {
                anim::set_playing(animations, *animation, *active);
            }
            HostEvent::SetTileAnimationRate {
                animation,
                frame_rate,
            } => anim::set_rate(animations, *animation, *frame_rate),
            HostEvent::SetTileAnimationFrameWait {
                animation,
                frame,
                wait,
            } => anim::set_frame_wait(animations, *animation, *frame, *wait),
            HostEvent::SetTileAnimationFlags { animation, flags } => {
                anim::add_flags(animations, *animation, *flags);
            }
            HostEvent::SetTileAnimationRange {
                animation,
                first_frame,
                last_frame,
            } => anim::set_range(animations, *animation, *first_frame, *last_frame),
            HostEvent::SetTileAnimationFrame { animation, frame } => {
                anim::show_frame(animations, *animation, *frame, &mut world.overlays);
            }
            HostEvent::ResetTileAnimation {
                animation,
                show_first_frame,
            } => anim::reset(
                animations,
                *animation,
                *show_first_frame != 0,
                &mut world.overlays,
            ),
            HostEvent::SetFieldCamera { camera } => {
                // `ActivateCamera` keeps the old camera for an unknown index.
                if let Ok(camera) = u8::try_from(*camera)
                    && camera < world.tile_camera_count
                {
                    world.field_camera = camera;
                }
            }
            _ => {}
        }
    }

    fn record_look(&mut self, event: &HostEvent) {
        // `PUSHHIDE` (0xD5) saves every object's flags and clears "show
        // model" (bit 1) unless "don't hide all" (32) is set; `POPSHOW`
        // (0xD6) restores bit 1 from the saved flags.
        match event {
            HostEvent::PreserveAndHideAll => {
                for state in self.world.actors.values_mut() {
                    state.look.saved_flags = state.flags;
                    if state.flags & 32 == 0 {
                        state.flags &= !1;
                    }
                }
                return;
            }
            HostEvent::RestoreAllVisibility => {
                for state in self.world.actors.values_mut() {
                    state.flags = (state.flags & !1) | (state.look.saved_flags & 1);
                }
                return;
            }
            _ => {}
        }
        // `geoMeshHide`/`geoMeshShow` keep a 16-bit mask (`(UInt16)(1 << mesh)`).
        let mesh_bit = |mesh: i32| {
            u32::try_from(mesh)
                .ok()
                .and_then(|mesh| 1u32.checked_shl(mesh))
                .unwrap_or(0)
                & 0xFFFF
        };
        let (actor, apply): (EntityId, &dyn Fn(&mut super::world::ActorState)) = match event {
            // `MODEL` (0x2F) also sets "show model": `gExec.flags |= 1`.
            HostEvent::SetActorModel { actor, model, .. } => (*actor, &|state| {
                state.look.model = Some(*model);
                state.flags |= 1;
            }),
            HostEvent::SetIdleAnimation { actor, animation } => {
                (*actor, &|state| state.look.idle = Some(*animation))
            }
            HostEvent::SetWalkAnimation { actor, animation } => {
                (*actor, &|state| state.look.walk = Some(*animation))
            }
            HostEvent::SetRunAnimation { actor, animation } => {
                (*actor, &|state| state.look.run = Some(*animation))
            }
            HostEvent::SetLeftTurnAnimation { actor, animation } => {
                (*actor, &|state| state.look.turn_left = Some(*animation))
            }
            HostEvent::SetRightTurnAnimation { actor, animation } => {
                (*actor, &|state| state.look.turn_right = Some(*animation))
            }
            HostEvent::PlayAnimation { actor, animation } => {
                (*actor, &|state| state.look.playing = Some(*animation))
            }
            HostEvent::StopAnimation { actor } => (*actor, &|state| state.look.playing = None),
            HostEvent::HideObjectMeshes { object, mesh } => {
                let bit = mesh_bit(*mesh);
                (*object, &move |state| state.look.hidden_meshes |= bit)
            }
            HostEvent::ShowObjectMeshes { object, mesh } => {
                let bit = mesh_bit(*mesh);
                (*object, &move |state| state.look.hidden_meshes &= !bit)
            }
            _ => return,
        };
        if let Some(state) = self.world.actors.get_mut(&actor) {
            apply(state);
        }
        if let HostEvent::SetActorModel { eye_height, .. } = event {
            self.world.actor_presentation.entry(actor).or_default().eye = *eye_height;
        }
        if matches!(event, HostEvent::SetActorModel { .. })
            && self
                .world
                .actors
                .get(&actor)
                .is_some_and(|state| state.position != [0.0; 3])
        {
            // `AddFieldChar` snaps a newly modeled actor with
            // `SetPosition` (pathing on by default).
            self.world.snap_actor_to_walkmesh(actor);
        }
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
            self.record_look(event);
            self.record_overlay(event);
            self.record_overlay_motion(event);
            self.record_light(event);
            if let HostEvent::SetFieldCamera { camera } = event {
                self.world.view.set_camera(*camera);
            }
            if let HostEvent::CalculateScreenOrigin = event {
                // `VRP` (EventEngine.DoEventCode.cs): `EBG_sceneGetVRP` into
                // `sSysX`/`sSysY`, read back as system variables 12 and 13.
                self.world.view.screen = self.world.view.origin();
            }
            if let HostEvent::SetMenuControl { enabled } = event {
                self.world.menu_disabled = !*enabled;
            }
            if let HostEvent::OpenMenu {
                menu: 4,
                subject: 0,
            } = event
            {
                // A moogle or save point: the reference opens the save screen.
                self.world.save_point = true;
                self.world.save_requested = true;
            }
            if let HostEvent::OpenMenu { menu: 2, subject } = event {
                // `EventService.OpenShopMenu`: the subject is the shop id.
                self.world.shop_requested = Some(*subject);
            }
            self.record_camera(event);
            self.record_tile_animation(event);
            match event {
                HostEvent::RunSound { .. }
                | HostEvent::SetSoundPosition { .. }
                | HostEvent::SetSoundObjectPosition { .. }
                | HostEvent::SetBattleMusicContinuation { .. } => {
                    self.world.sound_events.push(event.clone());
                }
                HostEvent::ShowWindow {
                    window,
                    flags,
                    text,
                    speaker,
                } => {
                    self.world
                        .window_speakers
                        .insert(*window, (*flags, *speaker));
                    self.world.windows.insert(*window, true);
                    self.world.window_texts.insert(*window, *text);
                    self.world.raise_text_signals(*text);
                }
                HostEvent::ShowObjectWindow {
                    object,
                    window,
                    flags,
                    text,
                } => {
                    self.world
                        .window_speakers
                        .insert(*window, (*flags, Some(*object)));
                    self.world.windows.insert(*window, true);
                    self.world.window_texts.insert(*window, *text);
                    self.world.raise_text_signals(*text);
                }
                HostEvent::SetTextVariable { script, value } => {
                    // MESVALUE (`ETb.gMesValue[scriptID] = value`).
                    self.world.mes_values.insert(*script, *value);
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
                        keep_facing: false,
                    });
                    state.exit_walk = false;
                }
                HostEvent::WalkActorMixed {
                    actor,
                    destination,
                    mode,
                } => {
                    // `MOVH` 0xA5 (slide), `MOVE3H` 0xE8 and 0xA2 all go through
                    // `EventEngine.MoveToward_mixed` (EventEngine.MoveToward.cs): a walk to
                    // (x, z) at the actor's speed, flag 1 keeping the facing. Field 1012 slides
                    // Zidane back out of its entry-14 region with `MOVH`; without the walk he
                    // stayed in the region and its scene replayed forever.
                    let state = self.actor_mut(*actor, "mixed walk")?;
                    state.movement = Some(Movement {
                        destination: [destination[0] as f32, destination[2] as f32],
                        keep_facing: mode & 1 != 0,
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
                            keep_facing: false,
                        });
                    } else if let Some(movement) = &mut state.movement {
                        movement.destination = [target_position[0], target_position[2]];
                    }
                }
                HostEvent::MoveActor { actor, x, z } => {
                    let state = self.actor_mut(*actor, "move")?;
                    // `DPOS`: `BGI_charSetActive(1)`, then `SetActorPosition`
                    // with the default height 32768.
                    state.position = [*x as f32, 32768.0, *z as f32];
                    state.path_active = true;
                    state.movement = None;
                    self.world.snap_actor_to_walkmesh(*actor);
                }
                HostEvent::MoveActorXzy { actor, position } => {
                    // `DPOS3`: `BGI_charSetActive(0)`; the position is kept.
                    let state = self.actor_mut(*actor, "move xzy")?;
                    state.position = [position[0] as f32, position[1] as f32, position[2] as f32];
                    state.path_active = false;
                    state.placed_unchecked = true;
                    state.movement = None;
                }
                HostEvent::SetPathing { actor, enabled } => {
                    // `BGI_charSetActive`.
                    let state = self.actor_mut(*actor, "set pathing")?;
                    state.path_active = *enabled > 0;
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
                    // `POS` passes the default height 32768 and turns
                    // pathing on; `POS3` passes a script height and turns
                    // it off (the event does not say which, the height does).
                    state.path_active = position[1] == 32768;
                    state.placed_unchecked = !state.path_active;
                    self.world.snap_actor_to_walkmesh(*actor);
                }
                HostEvent::DeleteObject { target } => {
                    self.world.delete_object(*target);
                }
                HostEvent::PlayAnimation { actor, animation } => {
                    // The clip's own length when the caller provides it.
                    let frames = self
                        .world
                        .animation_frame_counts
                        .as_ref()
                        .and_then(|counts| counts(*animation))
                        .unwrap_or(self.world.animation_default_ticks);
                    let state = self.actor_mut(*actor, "play animation")?;
                    // `ExecAnim` (EventEngine.cs:1313) starts on `inFrame`;
                    // a ranged animation lasts its frame span.
                    state.animation_frame = u16::from(state.in_frame);
                    state.animation_frames = if state.out_frame == u8::MAX {
                        frames
                    } else {
                        u16::from(state.out_frame.abs_diff(state.in_frame)).max(1)
                    };
                    state.anim_span = state.animation_frames;
                    state.anim_frozen = false;
                }
                HostEvent::PlayJumpAnimation { actor, landing } => {
                    // `PREJUMP` / `POSTJUMP` (DoEventCode.cs:2061-2076): the jump
                    // clip plays held from frame 0 to `jump0 - 1`, and the landing
                    // plays it from `jump1 + 1` to the end with the flags cleared.
                    let jump = self
                        .world
                        .actor_presentation
                        .get(actor)
                        .and_then(|presentation| presentation.jump_animation);
                    if let Some((animation, [jump0, jump1])) = jump {
                        let clip = self
                            .world
                            .animation_frame_counts
                            .as_ref()
                            .and_then(|counts| counts(animation))
                            .unwrap_or(self.world.animation_default_ticks);
                        let state = self.actor_mut(*actor, "play jump animation")?;
                        state.anim_hold = !*landing;
                        state.anim_loop = false;
                        state.anim_loops = 0;
                        state.in_frame = if *landing { jump1.wrapping_add(1) } else { 0 };
                        state.out_frame = if *landing {
                            u8::MAX
                        } else {
                            jump0.wrapping_sub(1)
                        };
                        state.animation_frame = u16::from(state.in_frame);
                        state.animation_frames = if *landing {
                            clip.saturating_sub(u16::from(state.in_frame)).max(1)
                        } else {
                            u16::from(state.out_frame).max(1)
                        };
                        state.anim_span = state.animation_frames;
                        state.anim_frozen = false;
                        state.look.playing = Some(animation);
                    }
                }
                HostEvent::SetAnimationFlags {
                    actor,
                    flags,
                    loop_count,
                } => {
                    // `AMODE` replaces the whole flag byte: hold 8, loop 16, palindrome 32.
                    let state = self.actor_mut(*actor, "set animation flags")?;
                    state.anim_hold = flags & 8 != 0;
                    state.anim_loop = flags & 48 != 0;
                    state.anim_loops = *loop_count;
                    state.anim_frozen = false;
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
                    let turn_frames = |left: bool, world: &super::world::FieldWorld| {
                        let state = world.actors.get(actor)?;
                        let id = if left {
                            state.look.turn_left
                        } else {
                            state.look.turn_right
                        }?;
                        world.animation_frame_counts.as_ref().and_then(|c| c(id))
                    };
                    // `StartTurn` (DoEventCode.cs:3340): a turn long enough to need its
                    // turn animation (more than 5 frames) restarts the animation flags
                    // (`animFlag = 0`), so a leftover loop or hold ends with it.
                    let previous = self.world.actors.get(actor).map(|state| state.facing);
                    let target = facing_from_fixed_point(*angle);
                    let long_turn = previous.is_some_and(|previous| {
                        let mut delta = f32::from((target - previous) & 0xFF) * 360.0 / 256.0;
                        if delta > 180.0 {
                            delta -= 360.0;
                        }
                        if delta.abs() < 0.01 {
                            return false;
                        }
                        let Some(frames) = turn_frames(delta < 0.0, self.world) else {
                            return false;
                        };
                        let speed = if *speed == 0 { 16 } else { *speed };
                        let mut count = (i32::from(frames) << 4) / speed;
                        if delta.abs() < 90.0 {
                            count = (count as f32 * delta.abs() / 90.0) as i32;
                        }
                        count > 5
                    });
                    let state = self.actor_mut(*actor, "turn")?;
                    if long_turn {
                        state.anim_hold = false;
                        state.anim_loop = false;
                        state.anim_loops = 0;
                    }
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
                HostEvent::SetMovementSpeed { actor, speed } => {
                    // `MSPEED` (DoEventCode.cs:886): `actor.speed = walkSpeed`. Memoria's fix
                    // for Quina in Qu's Marsh/Pond (map 658, object 18): a speed of 30 becomes
                    // 25 so Quina keeps pace with the scene (approved-differences
                    // `memoria-quina-658-walk-speed`; Memoria limits it to one script
                    // position, here it covers every speed-30 call of that object).
                    let speed =
                        if self.world.field_map_no == 658 && actor.get() == 18 && *speed == 30 {
                            25
                        } else {
                            *speed
                        };
                    self.actor_mut(*actor, "movement speed")?.move_speed = speed;
                }
                HostEvent::ResetWalkState { .. } => {
                    // `CLRDIST` (EventEngine.clrdist) resets the reference's walk-distance
                    // bookkeeping (`lastdist`, `actMove`); the walk here keeps no such state.
                }
                HostEvent::SetTurnSpeed { actor, speed } => {
                    // `TSPEED`: a speed of 0 means 16.
                    self.actor_mut(*actor, "turn speed")?.turn_speed =
                        if *speed == 0 { 16 } else { *speed };
                }
                HostEvent::SetWalkTurnSpeed { actor, speed } => {
                    // `MROT`: `actor.omega`; the script layer already mapped 0 to 255.
                    self.world
                        .actor_presentation
                        .entry(*actor)
                        .or_default()
                        .walk_turn_speed = *speed;
                }
                HostEvent::TurnTowardObject {
                    actor,
                    target,
                    speed,
                } => {
                    // `TURNA`: a missing object does nothing; otherwise `StartTurn` toward
                    // `angleAsm(target - actor)`.
                    let target_position = u64::try_from(*target)
                        .ok()
                        .map(|uid| self.world.resolve_object(EntityId::new(uid)))
                        .and_then(|entity| self.world.actors.get(&entity))
                        .map(|state| state.position);
                    if let Some(target_position) = target_position {
                        let state = self.actor_mut(*actor, "turn toward object")?;
                        let angle = angle_toward(state.position, target_position);
                        start_turn(state, angle, *speed);
                    }
                }
                HostEvent::TurnActorToPosition { actor, x, z } => {
                    // `TURNTO`: nothing when already standing on the point, else a turn at
                    // the object's own turn speed.
                    let state = self.actor_mut(*actor, "turn to position")?;
                    if state.position[0].round() as i32 != *x
                        || state.position[2].round() as i32 != *z
                    {
                        let angle = angle_toward(state.position, [*x as f32, 0.0, *z as f32]);
                        let speed = i32::from(state.turn_speed);
                        start_turn(state, angle, speed);
                    }
                }
                HostEvent::TurnToAngle { actor, angle } => {
                    // `TURNDS`: a turn at the object's own turn speed.
                    let state = self.actor_mut(*actor, "turn to angle")?;
                    let speed = i32::from(state.turn_speed);
                    start_turn(state, *angle, speed);
                }
                HostEvent::TurnObjectToAngle { actor, angle } => {
                    // `DDIR` (0x87): the object faces the angle at once (0 south .. 192 east).
                    let object = self.world.resolve_object(*actor);
                    let state = self.actor_mut(object, "turn object to angle")?;
                    state.facing = (*angle & 0xFF) as i16;
                }
                HostEvent::SetLeftTurnAnimation { actor, animation } => {
                    self.world
                        .actor_presentation
                        .entry(*actor)
                        .or_default()
                        .turn_left_animation = Some(*animation);
                }
                HostEvent::SetRightTurnAnimation { actor, animation } => {
                    self.world
                        .actor_presentation
                        .entry(*actor)
                        .or_default()
                        .turn_right_animation = Some(*animation);
                }
                HostEvent::SetJumpAnimation {
                    actor,
                    animation,
                    parameters,
                } => {
                    self.world
                        .actor_presentation
                        .entry(*actor)
                        .or_default()
                        .jump_animation = Some((*animation, *parameters));
                }
                HostEvent::SetInactiveAnimation { actor, animation } => {
                    self.world
                        .actor_presentation
                        .entry(*actor)
                        .or_default()
                        .inactive_animation = Some(*animation);
                }
                HostEvent::SetAnimationSpeed { actor, speed } => {
                    self.world
                        .actor_presentation
                        .entry(*actor)
                        .or_default()
                        .animation_speed = *speed;
                }
                HostEvent::SetAnimationLoopCount { actor, count } => {
                    self.world
                        .actor_presentation
                        .entry(*actor)
                        .or_default()
                        .animation_loop_count = *count;
                }
                HostEvent::DisableIdleAnimation => self.world.idle_disabled = true,
                HostEvent::AttachObject {
                    attached,
                    carrying,
                    bone,
                } => {
                    // `ATTACH` (0x4C, field mode): `geoAttach` needs both objects to exist;
                    // otherwise nothing happens.
                    let attached = self.world.resolve_object(*attached);
                    let carrying = self.world.resolve_object(*carrying);
                    if self.world.actors.contains_key(&attached)
                        && self.world.actors.contains_key(&carrying)
                    {
                        // `GeoAttach` copies the carrier's `AttachObjectOffset` (the script
                        // sets it on the carrier before attaching).
                        let offset = self
                            .world
                            .attachments
                            .get(&carrying)
                            .map_or([0; 3], |held| held.offset);
                        let entry = self.world.attachments.entry(attached).or_default();
                        entry.carrier = Some(carrying);
                        entry.bone = *bone;
                        entry.offset = offset;
                    }
                }
                HostEvent::DetachObject { attached } => {
                    // `DETACH` (0x4D): `geoDetach` puts the object back where it stands.
                    let attached = self.world.resolve_object(*attached);
                    if let Some(entry) = self.world.attachments.get_mut(&attached) {
                        entry.carrier = None;
                    }
                }
                HostEvent::AttachObjectOffset { actor, x, y, z } => {
                    // `ATTACHOFFSET` (0xD4): `geoAttachOffset` on the current object, the one
                    // a later `AttachObject` takes the offset from.
                    self.world.attachments.entry(*actor).or_default().offset = [*x, *y, *z];
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
                    let range = self
                        .world
                        .actor_presentation
                        .get(actor)
                        .and_then(|presentation| presentation.jump_animation)
                        .map(|(_, [jump0, jump1])| (jump0, jump1));
                    let state = self.actor_mut(*actor, "setup jump")?;
                    // `SETVY3` marks the jump and plays the jump clip over
                    // `jump0..jump1` (held by the earlier `PREJUMP`).
                    state.act_jump = true;
                    if let Some((jump0, jump1)) = range {
                        state.in_frame = jump0;
                        state.out_frame = jump1;
                        state.animation_frame = u16::from(jump0);
                        state.animation_frames = u16::from(jump1.abs_diff(jump0)).max(1);
                        state.anim_span = state.animation_frames;
                        state.anim_frozen = false;
                    }
                    // `JUMP3` setup: `BGI_charSetActive(0)` (EventEngine.DoEventCode.cs).
                    state.path_active = false;
                    state.placed_unchecked = true;
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
                    self.world.timer.set_time(*seconds);
                }
                HostEvent::SetTimerRunning { running } => {
                    self.world.timer.running = *running;
                }
                HostEvent::ShowTimer { show } => {
                    self.world.timer.displayed = *show;
                }
                HostEvent::SetRandomBattles { pattern, scenes } => {
                    self.world.random_battles.pattern = *pattern;
                    self.world.random_battles.scenes = *scenes;
                }
                HostEvent::SetRandomBattleFrequency { frequency } => {
                    self.world.random_battles.ratio = *frequency;
                    if *frequency == 0 {
                        self.world.random_battles.base = 0;
                    }
                }
                HostEvent::StartCardGame { opponent } => {
                    self.world.pending_card_game = Some(*opponent);
                }
                HostEvent::OpenPartyMenu {
                    minimum_party_size,
                    fixed_members,
                } => {
                    self.world
                        .open_party_menu_for(*minimum_party_size, *fixed_members);
                }
                HostEvent::StartBattle {
                    rush,
                    group,
                    battle,
                    steiner_state,
                } => {
                    // `ENCOUNT` clears the danger value.
                    self.world.random_battles.base = 0;
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
                HostEvent::RunWorldCode { code, value } => {
                    // `WPRM`: `ff9.w_frameSetParameter(code, value)`.
                    self.world.world_codes.push((*code, *value));
                }
                HostEvent::ChangeWorldMap { destination } => {
                    self.world.push_world_transition(*destination);
                }
                HostEvent::RequestScriptAsync { target, level, .. }
                | HostEvent::RequestScriptSync { target, level, .. } => {
                    self.world.pend_request(*target, *level);
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
                    // `NEW3`: a party slot (251-254) names its `partyUID`
                    // entry, and an empty slot creates nothing. uid 0
                    // defaults to the entry's own id; the object is the entry.
                    if let Some(source) = self.world.object_source(*source) {
                        let uid = if *unique_id == 0 { source } else { *unique_id };
                        self.world.init_object(source, uid);
                    }
                }
                HostEvent::SyncPartyUid => self.world.setup_party_uid(),
                HostEvent::SetCharacterRow { character, row } => {
                    // `SETROW`: `info.row` of the character (0 is the back row).
                    if let Some(character) = character_slot(*character) {
                        if *row == 0 {
                            self.world.back_row.insert(character);
                        } else {
                            self.world.back_row.remove(&character);
                        }
                    }
                }
                HostEvent::InitializeCharacter {
                    character,
                    enable_leveling,
                    equipment_set,
                    category,
                    menu_preset,
                } => {
                    // `JOIN` (`SetCharacterData`); 255 keeps the category and preset.
                    if let Some(character) = character_slot(*character) {
                        self.world
                            .character_info
                            .entry(character)
                            .or_default()
                            .initialized = Some(super::world::CharacterInit {
                            enable_leveling: *enable_leveling,
                            equipment_set: *equipment_set,
                            category: u8::try_from(*category).ok().filter(|v| *v != 255),
                            menu_preset: u8::try_from(*menu_preset).ok().filter(|v| *v != 255),
                        });
                    }
                }
                HostEvent::SetPlayerName { character, text } => {
                    // `PLAYERNAME`: the field text becomes the character's name.
                    if let Some(character) = character_slot(*character) {
                        self.world
                            .character_info
                            .entry(character)
                            .or_default()
                            .name_text = Some(*text);
                    }
                }
                HostEvent::ClearPartyStatus {
                    character,
                    statuses,
                } => {
                    // `CLEARSTATUS`: 0x7F is the vanilla "clear everything".
                    if let Some(character) = character_slot(*character) {
                        let mask = if *statuses == 0x7F {
                            u32::MAX
                        } else {
                            *statuses as u32
                        };
                        self.world
                            .character_info
                            .entry(character)
                            .or_default()
                            .cured_statuses |= mask;
                    }
                }
                HostEvent::SetPartyAvailability { mask } => {
                    // `FULLMEMBER`: the VM already moved Beatrix to bit 11.
                    self.world.party_reserve = Some(*mask);
                }
                HostEvent::GameOver => self.world.game_over = true,
                HostEvent::ChangeDisc { destination } => {
                    // `DISCCHANGE`: low 14 bits the field, bits 14-15 the disc.
                    self.world.disc_change = Some((
                        (*destination & 16383) as u16,
                        (*destination >> 14 & 3) as u8,
                    ));
                }
                // `HINT`: field preloading is ignored outside the PSX build.
                HostEvent::PreloadField { .. } => {}
                HostEvent::ExitField => {
                    // `MOVQ`: the controlled character walks to the exit and the player
                    // loses control.
                    if let Some(actor) = self.world.control_actor
                        && let Some(state) = self.world.actors.get_mut(&actor)
                    {
                        state.exit_walk = true;
                        state.movement = None;
                    }
                    self.world.user_control = false;
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
                HostEvent::DisableRun => self.world.run_disabled = true,
                HostEvent::SetRunEnabled { enabled } => self.world.run_disabled = !*enabled,
                // `SetControlDirection` (0x67) calls `SetTwistAD(arrow,
                // analog)`; key walking turns by `twist.y`, the second
                // argument (FieldMapActorController.cs:712).
                HostEvent::SetControlDirection { analog, .. } => {
                    self.world.control_twist = (f32::from(*analog) + 1.0) / 256.0 * 360.0;
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
            // 6 GetGil: `party.gil`.
            6 => return Ok(i32::try_from(self.world.gil).unwrap_or(i32::MAX)),
            // 24 GetLastBattleId (`ff9.btlMapNo`), 27 GetBattleResult (`ff9.btl_result`).
            24 => return Ok(self.world.last_battle),
            27 => return Ok(i32::from(self.world.battle_result)),
            // 12 GetScreenCalculatedX, 13 GetScreenCalculatedY (`sSysX`/`sSysY`).
            12 => return Ok(self.world.view.screen[0]),
            13 => return Ok(self.world.view.screen[1]),
            // 17 GetTimerTime, 21 IsTimerShown (`TimerUI.Enable`).
            17 => return Ok(self.world.timer.time_int()),
            21 => return Ok(i32::from(self.world.timer.displayed)),
            14 => return Ok(self.world.fmv.frame),
            15 => return Ok(self.world.fmv.sync()),
            // 18 GetTetraMasterResult, 19 GetCardAmount (`GetSysvar.cs`: scripts read at most
            // 95 so a full collection still reads as one that can take a card).
            18 => return Ok(self.world.last_card_result),
            19 => return Ok(self.world.cards.len().min(95) as i32),
            _ => {}
        }
        // Engine state is zero-initialized in the reference, so unset
        // system variables read as zero rather than erroring.
        Ok(self.world.system_variable(variable).unwrap_or(0))
    }

    fn read_system_list(&mut self, index: u8) -> Result<i32, EngineError> {
        // The reference keeps eight zero-initialized lists (`index & 7`).
        Ok(self.world.system_list(index & 7).unwrap_or(0))
    }

    fn write_system_list(&mut self, index: u8, value: i32) -> Result<(), EngineError> {
        self.world.set_system_list(index & 7, value);
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
        // `B_PTR` pushes `obj.uid` (EBin.cs:1088): the uid a `NEW3` gave the
        // object. An extra instance (Hot and Cold's dig holes, uids 128 and
        // up) has an entity id that is not a uid, so push the uid itself;
        // an entry's own object keeps its entry id. 250 is the controlled object
        // (`EventEngine.GetUIDFromLocalUID`), as for object members.
        let entity = if u64::from(uid) == CONTROL_OBJECT_UID {
            self.world.control_actor.or(self.world.player())
        } else {
            self.world.resolve_uid(i32::from(uid))
        };
        Ok(entity.map(|entity| {
            if self.world.instance_source(entity).is_some() {
                i32::from(uid)
            } else {
                i32::try_from(entity.get()).unwrap_or(i32::MAX)
            }
        }))
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
        let position = self.actor_position(actor, "BGI character floor")?;
        // `B_BGIFLOOR` (EventEngine.DoCalcOperationExt.cs) -> `BGI.BGI_charGetInfo`: the floor
        // of the triangle the actor stands on (`FieldMapActorController.activeFloor`), not the
        // mesh's start floor. Burmecia 755 only drops its floor when Zidane is on floor 3.
        let floor = self
            .world
            .navigation_floor_at(position)
            .ok_or_else(|| Self::unknown("BGI character floor without a navigation mesh"))?;
        Ok(i32::from(floor))
    }

    fn read_character_max_mp(&mut self, character: i32) -> Result<i32, EngineError> {
        self.world
            .character_stats
            .get(&character)
            .map(|stats| stats.max_mp)
            .ok_or_else(|| Self::unknown("character max MP"))
    }

    fn party_contains(&mut self, character: i32) -> Result<bool, EngineError> {
        // `partychk`: scripts name characters by the old index.
        Ok(self
            .world
            .party
            .contains(&character_old_index_to_id(character)))
    }

    fn add_party_member(&mut self, character: i32) -> Result<bool, EngineError> {
        // `partyadd`; reference failure flag: false on success.
        // `chr2slot` maps anything outside the twelve characters to NONE,
        // and adding NONE fails.
        if !(0..=11).contains(&character) {
            return Ok(true);
        }
        let character = character_old_index_to_id(character);
        if self.world.party.len() >= 4 || self.world.party.contains(&character) {
            return Ok(true);
        }
        self.world.party.push(character);
        self.world.setup_party_uid();
        Ok(false)
    }

    fn remove_party_member(&mut self, character: i32) -> Result<(), EngineError> {
        // `PARTYDELETE`: the member's slot empties. The field keeps the party
        // compact, so a later `partyadd` joins at the end instead of
        // refilling the emptied slot.
        let character = character_old_index_to_id(character);
        self.world.party.retain(|member| *member != character);
        self.world.setup_party_uid();
        Ok(())
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
        level: i32,
    ) -> Result<bool, EngineError> {
        Ok(self.world.request_acceptable(target, level))
    }

    fn async_request_accepted(&mut self, target: EntityId, level: i32) -> bool {
        self.world.request_acceptable(target, level)
    }

    fn script_request_is_complete(&mut self, entity: EntityId) -> Result<bool, EngineError> {
        Ok(!self.world.request_pending(entity))
    }

    fn script_request_is_complete_at(
        &mut self,
        entity: EntityId,
        level: i32,
    ) -> Result<bool, EngineError> {
        Ok(!self.world.request_pending_at(entity, level))
    }

    fn object_uid(&mut self, entity: EntityId) -> Option<i32> {
        self.world.uid_of(entity)
    }

    fn control_object(&mut self) -> Option<EntityId> {
        self.world.control_actor.or(self.world.player())
    }

    fn party_object(&mut self, slot: u8) -> Option<EntityId> {
        let uid = i32::from(tantalus_legacy::event::party::PARTY_SLOT_FIRST + slot);
        self.world.party_slot_object(uid)
    }

    fn object_deleted(&mut self, entity: EntityId) -> bool {
        self.world.is_object_deleted(entity)
    }

    fn read_custom_variable(&mut self, _variable: u16) -> Result<i32, EngineError> {
        Err(Self::unknown("custom variable"))
    }

    fn instance_memory(&mut self, entity: EntityId) -> Option<&mut VariableMemory> {
        Some(self.world.instance_memory(entity))
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

/// `EventEngine.angleAsm` between two positions, as the 12-bit fixed-point angle the turn
/// events carry (facing scale 0 south, 64 west, 128 north, 192 east, shifted left four).
fn angle_toward(from: [f32; 3], to: [f32; 3]) -> i16 {
    let (dx, dz) = (to[0] - from[0], to[2] - from[2]);
    if dx == 0.0 && dz == 0.0 {
        return 0;
    }
    let degrees = dx.atan2(-dz).to_degrees();
    let facing = (128.0 + degrees * 128.0 / 180.0).round().rem_euclid(256.0) as i16;
    facing << 4
}

/// `StartTurn` (DoEventCode.cs:3339): a turn onto the angle it already faces does nothing;
/// a speed of 0 means 16.
fn start_turn(state: &mut crate::field::ActorState, angle: i16, speed: i32) {
    let facing = facing_from_fixed_point(angle);
    if facing == state.facing {
        return;
    }
    let speed = if speed == 0 { 16 } else { speed };
    state.in_frame = 0;
    state.out_frame = u8::MAX;
    state.facing = facing;
    state.turn = Some(Turn {
        target: angle,
        speed,
        remaining_ticks: 1 + speed.max(1) as u32,
    });
}

/// `chr2slot`: a script's character index as a character id; anything outside the twelve
/// characters is `CharacterId.NONE`.
fn character_slot(old: i32) -> Option<i32> {
    (0..=11)
        .contains(&old)
        .then(|| character_old_index_to_id(old))
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
    fn requests_follow_the_object_script_level() {
        let mut world = FieldWorld::new();
        let object = EntityId::new(16);
        assert!(world.request_acceptable(object, 6));
        world.pend_request(object, 2);
        assert!(world.request_acceptable(object, 1));
        assert!(!world.request_acceptable(object, 2));
        world.pend_request(object, 1);
        assert!(!world.request_acceptable(object, 1));
        world.complete_request(object);
        assert!(world.request_acceptable(object, 1));
        assert!(world.request_pending(object));
        world.clear_requests(object);
        assert!(!world.request_pending(object));
        assert!(world.request_acceptable(object, 6));
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

    #[test]
    fn model_hide_all_and_mesh_events_drive_what_the_renderer_draws() {
        let mut world = FieldWorld::new();
        let shown = EntityId::new(3);
        let kept = EntityId::new(4);
        world.add_actor(shown, [0.0; 3]);
        world.add_actor(kept, [0.0; 3]);
        let mut host = FieldHost::new(&mut world);
        host.apply(&[
            HostEvent::SetActorModel {
                actor: shown,
                model: 98,
                eye_height: 0,
            },
            HostEvent::SetIdleAnimation {
                actor: shown,
                animation: 7,
            },
            HostEvent::SetObjectFlags {
                object: kept,
                flags: 33,
            },
            HostEvent::HideObjectMeshes {
                object: shown,
                mesh: 1,
            },
            HostEvent::PreserveAndHideAll,
        ])
        .unwrap();
        let look = world.actors[&shown].look;
        assert_eq!(
            (look.model, look.idle, look.hidden_meshes),
            (Some(98), Some(7), 2)
        );
        // `MODEL` set "show model"; hide-all cleared it except on the
        // "don't hide all" (32) object.
        assert_eq!(world.actors[&shown].flags & 1, 0);
        assert_eq!(world.actors[&kept].flags & 1, 1);
        let mut host = FieldHost::new(&mut world);
        host.apply(&[
            HostEvent::RestoreAllVisibility,
            HostEvent::ShowObjectMeshes {
                object: shown,
                mesh: 1,
            },
        ])
        .unwrap();
        assert_eq!(world.actors[&shown].flags & 1, 1);
        assert_eq!(world.actors[&shown].look.hidden_meshes, 0);
    }

    #[test]
    fn screen_origin_reads_the_scrolled_view() {
        // Field 606 (Lindblum telescope): `SCROLL` by 3 pixels a frame, then `VRP` and
        // `GetScreenCalculatedX/Y` (system variables 12 and 13) read the view back.
        let mut world = FieldWorld::new();
        world.provide_cameras(vec![tantalus_contracts::LegacyFieldCamera {
            projection: 256,
            rotation: [4096, 0, 0, 0, 4096, 0, 0, 0, 4096],
            translation: tantalus_contracts::Point3 {
                x: 0,
                y: 0,
                z: 1000,
            },
            center_offset: [0, 0],
            size: [512, 336],
            viewport_x: [100, 412],
            viewport_y: [99, 238],
            depth_offset: 0,
        }]);
        let mut host = FieldHost::new(&mut world);
        assert_eq!(host.read_system_variable(12).unwrap(), 0);
        host.apply(&[HostEvent::CalculateScreenOrigin]).unwrap();
        assert_eq!(host.read_system_variable(12).unwrap(), 256);
        assert_eq!(host.read_system_variable(13).unwrap(), 168);
        host.apply(&[HostEvent::MoveCamera {
            x: 253,
            y: 171,
            duration: 1,
            mode: 8,
        }])
        .unwrap();
        world.service_view();
        let mut host = FieldHost::new(&mut world);
        host.apply(&[HostEvent::CalculateScreenOrigin]).unwrap();
        assert_eq!(host.read_system_variable(12).unwrap(), 253);
        assert_eq!(host.read_system_variable(13).unwrap(), 171);
    }

    #[test]
    fn the_last_battle_and_its_result_read_back() {
        let mut world = FieldWorld::new();
        world.last_battle = 35;
        world.battle_result = 3;
        let mut host = FieldHost::new(&mut world);
        assert_eq!(host.read_system_variable(24).unwrap(), 35);
        assert_eq!(host.read_system_variable(27).unwrap(), 3);
    }

    #[test]
    fn tile_animation_opcodes_drive_the_overlay_service() {
        use crate::field::tile_animation::TileAnimation;
        let mut world = FieldWorld::new();
        world.provide_tile_animations(vec![TileAnimation::from_file(0, vec![(7, 1), (8, 1)])], 2);
        FieldHost::new(&mut world)
            .apply(&[
                HostEvent::SetTileAnimationActive {
                    animation: 0,
                    active: true,
                },
                HostEvent::SetTileAnimationFlags {
                    animation: 0,
                    flags: 16,
                },
            ])
            .unwrap();
        world.service_tile_animations();
        assert_eq!(world.overlays[&7].active, Some(true));
        assert_eq!(world.overlays[&8].active, Some(false));
        world.service_tile_animations();
        assert_eq!(world.overlays[&7].active, Some(false));
        assert_eq!(world.overlays[&8].active, Some(true));
        // A camera the animation does not belong to pauses it.
        FieldHost::new(&mut world)
            .apply(&[HostEvent::SetFieldCamera { camera: 1 }])
            .unwrap();
        assert_eq!(world.field_camera, 1);
        world.service_tile_animations();
        assert_eq!(world.overlays[&8].active, Some(true));
        // An unknown camera keeps the current one.
        FieldHost::new(&mut world)
            .apply(&[HostEvent::SetFieldCamera { camera: 5 }])
            .unwrap();
        assert_eq!(world.field_camera, 1);
        FieldHost::new(&mut world)
            .apply(&[
                HostEvent::SetFieldCamera { camera: 0 },
                HostEvent::SetTileAnimationActive {
                    animation: 0,
                    active: false,
                },
            ])
            .unwrap();
        world.service_tile_animations();
        assert_eq!(world.overlays[&8].active, Some(true));
        world.begin_field();
        assert!(world.tile_animations.is_empty());
    }

    #[test]
    fn layer_scroll_scripts_are_logged_for_the_renderer_until_the_field_ends() {
        use super::super::world::OverlayMotion;
        let mut world = FieldWorld::new();
        FieldHost::new(&mut world)
            .apply(&[
                HostEvent::SetTileParallax {
                    block: 19,
                    enabled: 1,
                    x_speed: 32,
                    y_speed: 0,
                },
                HostEvent::MoveTile {
                    overlay: 5,
                    offset: [160, 100, 0],
                },
            ])
            .unwrap();
        assert_eq!(
            world.overlay_motion,
            vec![
                OverlayMotion::Parallax {
                    block: 19,
                    enabled: true,
                    speed: [32, 0]
                },
                OverlayMotion::Move {
                    block: 5,
                    offset: [160, 100, 0]
                },
            ]
        );
        world.begin_field();
        assert!(world.overlay_motion.is_empty());
    }

    #[test]
    fn tile_block_events_are_kept_for_the_renderer_until_the_field_ends() {
        let mut world = FieldWorld::new();
        let mut host = FieldHost::new(&mut world);
        host.apply(&[
            HostEvent::SetTileBlockActive {
                block: 16,
                active: false,
            },
            HostEvent::AttachTile {
                overlay: 17,
                x: 240,
                y: 136,
                filter_mode: 0,
                color: [0, 0, 0],
            },
            HostEvent::SetTileOrigin {
                block: 3,
                x: -8,
                y: 4,
            },
            HostEvent::AttachTile {
                overlay: 5,
                x: 1,
                y: 2,
                filter_mode: -1,
                color: [9, 9, 9],
            },
        ])
        .unwrap();
        assert_eq!(world.overlays[&16].active, Some(false));
        assert_eq!(world.overlays[&17].attached, Some([240, 136]));
        assert_eq!(world.overlays[&17].border, Some([0, 0, 0]));
        assert_eq!(world.overlays[&3].origin, Some([-8, 4]));
        // Filter mode -1: no border.
        assert_eq!(world.overlays[&5].border, None);
        world.begin_field();
        assert!(world.overlays.is_empty());
    }
}
