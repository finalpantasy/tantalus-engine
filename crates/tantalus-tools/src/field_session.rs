//! Running one field's event scripts the way `EventEngine` schedules them:
//! the main object's init on load, each created object's init then loop,
//! script requests, shared scripts (`STARTSEQ`), `DRET`, talk and region
//! functions. [`EntryScheduler`] holds the bookkeeping so the trace's
//! scripted player and the live client share one implementation;
//! [`FieldSession`] bundles it with the director for live play.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use tantalus_contracts::EntityId;
use tantalus_game::field::{DirectorStep, FieldDirector, FieldWorld};
use tantalus_legacy::{EventBinary, EventProgram, VmStatus};

/// Every function of every entry of one field's event script, by entry:
/// `(tag, program)`, empty functions dropped.
pub type Pool = Vec<Vec<(u16, EventProgram)>>;

/// Director keys for shared scripts (`STARTSEQ`) start here, one per
/// object, so they never collide with `entry * 256 + function` keys.
pub const SHARED_SCRIPT_KEY: u64 = 0x1_0000;

/// `EventInput.Confirm`.
pub const CONFIRM: i32 = 0x20000;

/// Reads the `name` TextAsset (an `.eb` event script) from a Unity bundle.
pub fn load_event(bundle_path: &Path, name: &str) -> Result<EventBinary, String> {
    let script = find_text_asset(bundle_path, name)?
        .ok_or_else(|| format!("event asset `{name}` was not found"))?;
    EventBinary::parse(&script).map_err(|error| error.to_string())
}

/// The bytes of the TextAsset called `name` in a Unity bundle, if any.
pub fn find_text_asset(bundle_path: &Path, name: &str) -> Result<Option<Vec<u8>>, String> {
    let bundle = tantalus_assets::pc::unity_raw::UnityRawBundle::open(bundle_path)
        .map_err(|error| error.to_string())?;
    let bytes = bundle
        .read_entry(0, 300 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    let serialized = tantalus_assets::pc::serialized::SerializedFile::parse(
        &bytes,
        &bundle_path.to_string_lossy(),
    )
    .map_err(|error| error.to_string())?;
    for (index, object) in serialized.report().objects.iter().enumerate() {
        if object.class_id != 49 {
            continue;
        }
        let asset = tantalus_assets::pc::text_asset::TextAsset::parse(
            serialized
                .object_data(index)
                .map_err(|error| error.to_string())?,
            name,
        )
        .map_err(|error| error.to_string())?;
        if asset.name == name {
            return Ok(Some(asset.script));
        }
    }
    Ok(None)
}

/// Decodes every entry; returns the pool and the number of decode errors.
pub fn decode_pool(binary: &EventBinary) -> Result<(Pool, usize), String> {
    let mut pool: Pool = Vec::new();
    let mut decode_errors = 0usize;
    for entry in &binary.entries {
        let decoded = binary
            .decode_entry_with_diagnostics(entry.index)
            .map_err(|error| error.to_string())?;
        decode_errors += decoded.errors.len();
        pool.push(
            decoded
                .functions
                .into_iter()
                .filter(|function| !function.program.instructions.is_empty())
                .map(|function| (function.tag, function.program))
                .collect(),
        );
    }
    Ok((pool, decode_errors))
}

/// Registers one function of `entry` as a scene VM (key = entry * 256 +
/// function ordinal) acting on the entry's object. Returns `None` when the
/// tag is absent or the VM is already registered.
pub fn register(
    director: &mut FieldDirector,
    pool: &Pool,
    entry: usize,
    tag: Option<u16>,
) -> Result<Option<EntityId>, String> {
    let functions = &pool[entry];
    let position = match tag {
        Some(tag) => functions.iter().position(|(t, _)| *t == tag),
        None => (!functions.is_empty()).then_some(0),
    };
    let Some(position) = position else {
        return Ok(None);
    };
    let entity = EntityId::new(entry as u64 * 256 + position as u64);
    if director
        .entry_states()
        .iter()
        .any(|(existing, _, _, _)| *existing == entity)
    {
        return Ok(None);
    }
    // Every function of an entry acts on the entry's object (one `Obj` in
    // the reference), registered per field as `EntityId(entry)` with uid
    // `entry`.
    director
        .add_object_function(
            entity,
            EntityId::new(entry as u64),
            functions[position].1.clone(),
        )
        .map_err(|error| error.to_string())?;
    Ok(Some(entity))
}

/// The director key of `entry`'s function `tag`, if the entry has it.
pub fn function_entity(pool: &Pool, entry: usize, tag: u16) -> Option<EntityId> {
    let position = pool[entry].iter().position(|(t, _)| *t == tag)?;
    Some(EntityId::new(entry as u64 * 256 + position as u64))
}

fn is_completed(director: &FieldDirector, entity: EntityId) -> bool {
    director
        .entry_states()
        .iter()
        .any(|(e, _, status, _)| *e == entity && *status == VmStatus::Completed)
}

/// Starts `entry`'s function `tag` again: a finished run is dropped first,
/// since each call starts from the top. `Ok(true)` when it was registered.
pub fn rerun(
    director: &mut FieldDirector,
    pool: &Pool,
    entry: usize,
    tag: u16,
) -> Result<bool, String> {
    if entry >= pool.len() {
        return Ok(false);
    }
    if let Some(entity) = function_entity(pool, entry, tag)
        && is_completed(director, entity)
    {
        director.remove_entry(entity);
    }
    Ok(register(director, pool, entry, Some(tag))?.is_some())
}

/// The field-load half of `EventEngine.StartEvents` for `world`: pending
/// transitions dropped, every window closed, one actor per entry (uid =
/// entry) plus the party slots 251-255.
pub fn new_director(mut world: FieldWorld, entries: usize, held: i32) -> FieldDirector {
    world.take_field_transitions();
    world.windows.clear();
    world.window_texts.clear();
    let mut director = FieldDirector::new(world);
    director.world_mut().set_input(held);
    for entry in 0..entries {
        let base = EntityId::new(entry as u64);
        director.world_mut().add_actor(base, [0.0; 3]);
        director.world_mut().map_uid(entry as i32, base);
    }
    for slot in 251u64..256 {
        let entity = EntityId::new(slot);
        director.world_mut().add_actor(entity, [0.0; 3]);
        director.world_mut().map_uid(slot as i32, entity);
    }
    director
}

/// Which functions are running for whom, between director steps.
#[derive(Debug, Default)]
pub struct EntryScheduler {
    /// Objects still in their init function (tag 0). When it returns,
    /// `EventEngine.Return` (EventEngine.cs:1178) moves the object to
    /// stateRunning and starts its loop function (tag 1) at level 7.
    initializing: Vec<(EntityId, usize)>,
    /// Shared script key -> the object it runs for.
    shared_running: BTreeMap<EntityId, EntityId>,
    /// Requested functions still running, with the target entry.
    spawned: Vec<(EntityId, u64)>,
    pub loops_started: usize,
    pub shared_scripts_run: usize,
    /// Requests with no function to run.
    pub unspawned: usize,
}

impl EntryScheduler {
    /// Field load starts entry 0 only (`EventEngine.ProcessEvents` runs the
    /// main object's init); the rest start when a script creates them.
    pub fn start(director: &mut FieldDirector, pool: &Pool) -> Result<Self, String> {
        let mut scheduler = Self::default();
        if !pool.is_empty()
            && let Some(entity) = register(director, pool, 0, Some(0))?
        {
            scheduler.initializing.push((entity, 0));
        }
        Ok(scheduler)
    }

    /// Requested functions still running.
    pub fn spawned_pending(&self) -> usize {
        self.spawned.len()
    }

    /// Objects a script created this step (`DirectorStep::initialized`):
    /// their init function, or their loop when they have no init.
    pub fn start_created(
        &mut self,
        director: &mut FieldDirector,
        pool: &Pool,
        step: &DirectorStep,
    ) -> Result<(), String> {
        for source in &step.initialized {
            let entry = usize::from(*source);
            if entry >= pool.len() {
                continue;
            }
            let first = if pool[entry].iter().any(|(tag, _)| *tag == 0) {
                0
            } else {
                1
            };
            if let Some(entity) = register(director, pool, entry, Some(first))? {
                if first == 0 {
                    self.initializing.push((entity, entry));
                } else {
                    self.loops_started += 1;
                }
            }
        }
        Ok(())
    }

    /// Shared scripts, `DRET` and requests from this step.
    pub fn run_step_scripts(
        &mut self,
        director: &mut FieldDirector,
        pool: &Pool,
        step: &DirectorStep,
    ) -> Result<(), String> {
        // STARTSEQ: a new shared script replaces the object's previous one
        // (`DisposeObj` of uid + UID_OFFSET_SEQ).
        for (object, entry) in &step.shared_scripts {
            let Ok(entry) = usize::try_from(*entry) else {
                continue;
            };
            let Some((_, program)) = pool.get(entry).and_then(|f| f.first()) else {
                continue;
            };
            let key = EntityId::new(SHARED_SCRIPT_KEY + object.get());
            director.remove_entry(key);
            director
                .add_shared_script(key, *object, program.clone())
                .map_err(|error| error.to_string())?;
            self.shared_running.insert(key, *object);
            self.shared_scripts_run += 1;
        }
        // DRET: the object's running functions all return.
        for object in &step.returned {
            self.spawned.retain(|(entity, base)| {
                let keep = *base != object.get();
                if !keep {
                    director.remove_entry(*entity);
                }
                keep
            });
        }
        for request in &step.requests {
            let target = request.target.get() as usize;
            if target >= pool.len() {
                self.unspawned += 1;
                continue;
            }
            let tag = u16::from(request.tag);
            let tag = pool[target].iter().any(|(t, _)| *t == tag).then_some(tag);
            // A function that already ran to its end runs again from its
            // start (each request is a fresh call).
            if let Some(entity) = tag.and_then(|tag| function_entity(pool, target, tag))
                && is_completed(director, entity)
            {
                director.remove_entry(entity);
            }
            match register(director, pool, target, tag)? {
                Some(entity) => self.spawned.push((entity, target as u64)),
                None => {
                    // Nothing will run, so nothing would ever clear the
                    // pend: release a waiting caller.
                    self.unspawned += 1;
                    if !self.spawned.iter().any(|(_, base)| *base == target as u64) {
                        director
                            .world_mut()
                            .complete_request(EntityId::new(target as u64));
                    }
                }
            }
        }
        Ok(())
    }

    /// After a step: finished shared scripts release their object,
    /// finished requests complete, finished inits start their loops.
    pub fn settle(&mut self, director: &mut FieldDirector, pool: &Pool) -> Result<(), String> {
        let states = director.entry_states();
        let done_entity = |entity: &EntityId| {
            states
                .iter()
                .any(|(e, _, status, _)| e == entity && *status == VmStatus::Completed)
        };
        let mut finished_shared = Vec::new();
        self.shared_running.retain(|key, object| {
            let complete = done_entity(key);
            if complete {
                finished_shared.push(*object);
            }
            !complete
        });
        let mut done = Vec::new();
        self.spawned.retain(|(entity, base)| {
            let complete = done_entity(entity);
            if complete {
                done.push(*base);
            }
            !complete
        });
        let mut ready = Vec::new();
        self.initializing.retain(|(entity, entry)| {
            let complete = done_entity(entity);
            if complete {
                ready.push(*entry);
            }
            !complete
        });
        for object in finished_shared {
            director.world_mut().finish_shared_script(object);
        }
        for base in done {
            director.world_mut().complete_request(EntityId::new(base));
        }
        for entry in ready {
            if pool[entry].iter().any(|(tag, _)| *tag == 1)
                && register(director, pool, entry, Some(1))?.is_some()
            {
                self.loops_started += 1;
            }
        }
        Ok(())
    }
}

/// Back on the same field after a battle, `EventEngine.StartEvents`
/// restores the saved context and requests the main object's function 10
/// at level 0 (`EventEngine.cs:598`).
pub fn resume_after_battle(director: &mut FieldDirector, pool: &Pool) -> Result<(), String> {
    if pool.is_empty() {
        return Ok(());
    }
    rerun(director, pool, 0, 10)?;
    Ok(())
}

/// Whether any talk function (tag 3) is still running: one confirm starts
/// one talk, and none while another runs.
pub fn talk_busy(director: &FieldDirector, pool: &Pool) -> bool {
    let states = director.entry_states();
    (0..pool.len()).any(|entry| {
        function_entity(pool, entry, 3).is_some_and(|entity| {
            states
                .iter()
                .any(|(e, _, status, _)| *e == entity && *status != VmStatus::Completed)
        })
    })
}

/// What one live tick produced.
#[derive(Debug, Default)]
pub struct SessionTick {
    /// Battles the field started (`DirectorStep::encounters`).
    pub encounters: Vec<tantalus_game::field::EncounterRequest>,
    /// Field the scripts changed to.
    pub next_field: Option<i32>,
    /// World map the scripts changed to.
    pub world_map: Option<i32>,
    /// Entries whose talk (tag 3) or region (tag 2) function started.
    pub started: Vec<(usize, u16)>,
}

/// One field, live: the director, its scripts and the scheduler, with
/// region collision (`EventCollision`: standing in a region runs its tag 2)
/// and talk on confirm (tag 3 of the object the director reports).
pub struct FieldSession {
    pub map: i32,
    pub director: FieldDirector,
    pub pool: Pool,
    scheduler: EntryScheduler,
    inside: BTreeSet<usize>,
}

impl FieldSession {
    /// Starts field `map` on `world`, which the caller has prepared
    /// ([`prepare_field_world`], navigation, text).
    pub fn start(map: i32, world: FieldWorld, pool: Pool) -> Result<Self, String> {
        let mut director = new_director(world, pool.len(), 0);
        director.set_confirm_mask(Some(CONFIRM));
        let scheduler = EntryScheduler::start(&mut director, &pool)?;
        Ok(Self {
            map,
            director,
            pool,
            scheduler,
            inside: BTreeSet::new(),
        })
    }

    /// One tick with `input` held (`EventInput` bits).
    pub fn tick(&mut self, input: i32) -> Result<SessionTick, String> {
        self.director.world_mut().set_input(input);
        let step = self.director.step().map_err(|error| error.to_string())?;
        self.scheduler
            .start_created(&mut self.director, &self.pool, &step)?;
        let mut tick = SessionTick {
            encounters: step.encounters.clone(),
            ..SessionTick::default()
        };
        let mut busy = talk_busy(&self.director, &self.pool);
        for trigger in &step.talk_triggers {
            let entry = trigger.get() as usize;
            if busy {
                break;
            }
            if rerun(&mut self.director, &self.pool, entry, 3)? {
                busy = true;
                tick.started.push((entry, 3));
            }
        }
        self.scheduler
            .run_step_scripts(&mut self.director, &self.pool, &step)?;
        self.scheduler.settle(&mut self.director, &self.pool)?;
        self.collide(&mut tick)?;
        tick.world_map = self
            .director
            .world_mut()
            .take_world_transitions()
            .first()
            .map(|map| i32::from(*map));
        tick.next_field = self
            .director
            .world_mut()
            .take_field_transitions()
            .first()
            .map(|map| i32::from(*map));
        Ok(tick)
    }

    /// `EventCollision.CollisionRequest`: every tick the player has
    /// control and stands in a region, that region's tag 2 is requested
    /// again (`TreadQuad` + `Request(quad, 1, 2)`); a run still going is
    /// left alone. Only the first request after entering is reported.
    fn collide(&mut self, tick: &mut SessionTick) -> Result<(), String> {
        let world = self.director.world();
        let Some(position) = world
            .player()
            .and_then(|player| world.actors.get(&player))
            .map(|actor| actor.position)
        else {
            return Ok(());
        };
        let now: BTreeSet<usize> =
            tantalus_game::field::region::regions_at(&world.regions, position[0], position[2])
                .into_iter()
                .map(|region| region.get() as usize)
                .filter(|entry| {
                    *entry < self.pool.len() && self.pool[*entry].iter().any(|(t, _)| *t == 2)
                })
                .collect();
        let control = world.user_control;
        let entered: BTreeSet<usize> = now.difference(&self.inside).copied().collect();
        // `TreadQuad` picks one region the player stands in.
        let tread = now.iter().next().copied();
        self.inside = now;
        if !control {
            return Ok(());
        }
        if let Some(entry) = tread
            && rerun(&mut self.director, &self.pool, entry, 2)?
            && entered.contains(&entry)
        {
            tick.started.push((entry, 2));
        }
        Ok(())
    }

    /// Call after the field's battles were fought (or skipped).
    pub fn resume_after_battle(&mut self) -> Result<(), String> {
        resume_after_battle(&mut self.director, &self.pool)
    }
}

/// The per-field resets `EventEngine.InitEvents` does before the scripts
/// start: the field number, map variables zeroed (EventEngine.cs:556), and
/// the world's field-scoped state.
pub fn prepare_field_world(world: &mut FieldWorld, map: i32) {
    world.field_map_no = map;
    world.map_memory.clear();
    world.begin_field();
}

/// The world map stand-in. The world script (`EVT_WORLD_WORLD00` for map
/// 9000) leaves for a field through one `JumpSwitch` on the location the
/// party stands at (map variable 29, copied from 24) whose cases end in
/// `ChangeField`. This runs that code with the real VM: it sets the
/// location, starts after the exit fade (`ApplyFadeFilter`) that precedes
/// the switch, and returns the field the script changes to. Walking the
/// world, encounters and the other world opcodes are not modeled.
pub fn world_exit(
    bundle: &Path,
    name: &str,
    location: i32,
    world: &mut tantalus_game::field::FieldWorld,
) -> Result<Option<i32>, String> {
    use tantalus_legacy::{InstructionKind, VariableType};
    let binary = load_event(bundle, &format!("{name}.eb"))?;
    let (pool, _) = decode_pool(&binary)?;
    let program = pool
        .iter()
        .flatten()
        .map(|(_, program)| program)
        .find(|program| {
            let has_switch = program
                .instructions
                .values()
                .any(|i| matches!(i.kind, InstructionKind::JumpSwitch { .. }));
            let changes = program
                .instructions
                .values()
                .filter(|i| matches!(i.kind, InstructionKind::ChangeField(_)))
                .count();
            has_switch && changes > 8
        })
        .ok_or("no world exit switch in the world script")?;
    let switch = program
        .instructions
        .values()
        .filter(|i| matches!(i.kind, InstructionKind::JumpSwitch { .. }))
        .max_by_key(|i| match &i.kind {
            InstructionKind::JumpSwitch { case_relative, .. } => case_relative.len(),
            _ => 0,
        })
        .ok_or("no switch")?;
    let start = program
        .instructions
        .range(..switch.offset)
        .rev()
        .find(|(_, i)| matches!(i.kind, InstructionKind::ApplyFadeFilter { .. }))
        .and_then(|(offset, _)| program.instructions.range(offset + 1..).next())
        .map_or(switch.offset, |(offset, _)| *offset);
    world
        .map_memory
        .set(29, VariableType::Byte, location)
        .ok_or("map variable 29 out of range")?;
    world
        .map_memory
        .set(24, VariableType::Byte, location)
        .ok_or("map variable 24 out of range")?;
    let mut vm = tantalus_legacy::EventVm::resume(
        program.clone(),
        EntityId::new(0),
        start,
        VmStatus::Running,
        None,
    )
    .map_err(|error| error.to_string())?;
    let mut host = tantalus_game::field::FieldHost::new(world);
    for tick in 0..600u64 {
        let report = vm
            .step(tantalus_contracts::Tick::new(tick), &mut host)
            .map_err(|error| error.to_string())?;
        for event in &report.events {
            if let tantalus_legacy::HostEvent::ChangeField { field } = event {
                return Ok(Some(i32::from(*field)));
            }
        }
        if report.status == VmStatus::Completed {
            return Ok(None);
        }
    }
    Err("world exit code did not reach ChangeField in 600 ticks".to_owned())
}
