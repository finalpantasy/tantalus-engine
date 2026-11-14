use std::time::Duration;
use tantalus_contracts::*;
use tantalus_core::{PresentationClock, ResourceRequestStatus, Simulation};

fn config(progression: ProgressionMode) -> SimulationConfig {
    SimulationConfig {
        rate: SimulationRate::per_second(60).unwrap(),
        progression,
        content_revision: ContentRevision::from_u64(7),
        session_id: 11,
        max_step_stride: 16,
        rng: RngConfig::default(),
        diagnostics: DiagnosticPolicy {
            hash_every_step: true,
            detailed_traces: true,
        },
    }
}

fn id(text: &str) -> ContentId {
    ContentId::parse(text).unwrap()
}

fn button(control: &str, transition: ButtonTransition) -> GameCommand {
    GameCommand::ButtonTransition {
        control: control.to_owned(),
        transition,
    }
}

fn request(number: u64) -> GameCommand {
    GameCommand::RequestResource {
        request: ResourceRequestId::new(number),
    }
}

fn ready(tick: u64, number: u64) -> RecordedExternalEvent {
    RecordedExternalEvent {
        tick: Tick::new(tick),
        sequence: 0,
        session_id: 11,
        content_revision: ContentRevision::from_u64(7),
        request: ResourceRequestId::new(number),
        kind: ExternalEventKind::Ready,
    }
}

fn effect_command() -> GameCommand {
    GameCommand::StartEffect {
        effect: EntityId::new(9),
        timeline: EffectTimeline {
            duration_ticks: 6,
            markers: vec![
                TimelineMarker {
                    tick_offset: 2,
                    event: id("fixture:effect/windup"),
                },
                TimelineMarker {
                    tick_offset: 4,
                    event: id("fixture:effect/first_hit"),
                },
                TimelineMarker {
                    tick_offset: 4,
                    event: id("fixture:effect/second_hit"),
                },
                TimelineMarker {
                    tick_offset: 6,
                    event: id("fixture:effect/final_hit"),
                },
            ],
            on_complete: id("fixture:callback/effect_complete"),
        },
    }
}

fn commands_between(start: u64, end: u64) -> Vec<GameCommand> {
    let mut commands = Vec::new();
    if start < 1 && end >= 1 {
        commands.push(button("confirm", ButtonTransition::Press));
        commands.push(button("confirm", ButtonTransition::Release));
    }
    if start < 2 && end >= 2 {
        commands.push(request(1));
    }
    if start < 4 && end >= 4 {
        commands.push(effect_command());
    }
    commands
}

fn events_between(start: u64, end: u64) -> Vec<RecordedExternalEvent> {
    if start < 3 && end >= 3 {
        vec![ready(3, 1)]
    } else {
        Vec::new()
    }
}

fn run_at_presentation_rate(fps: u32) -> (StateDigest, Vec<DomainEvent>) {
    let mut clock = PresentationClock::new(SimulationRate::per_second(60).unwrap(), 64).unwrap();
    let mut simulation = Simulation::new(config(ProgressionMode::ContinuingWorld)).unwrap();
    let mut events = Vec::new();

    for _ in 0..2_000 {
        let advance = clock
            .advance(Duration::from_secs_f64(1.0 / f64::from(fps)))
            .unwrap();
        for mut input in advance.inputs {
            let end = input.tick.get();
            let start = end - u64::from(input.stride);
            input.commands = commands_between(start, end);
            input.external_events = events_between(start, end);
            let result = simulation.step(input).unwrap();
            events.extend(result.events.clone());
            if result.tick == Tick::new(10) {
                return (result.state_digest.expect("verification digest"), events);
            }
        }
    }
    panic!("presentation schedule did not reach tick 10");
}

#[test]
fn replay_matches_across_six_presentation_schedules() {
    let expected = run_at_presentation_rate(60);
    assert_eq!(expected.1.len(), 10);

    for fps in [30, 60, 120, 144, 165, 240] {
        let actual = run_at_presentation_rate(fps);
        assert_eq!(actual.0, expected.0, "state digest differed at {fps} FPS");
        assert_eq!(actual.1, expected.1, "domain events differed at {fps} FPS");
    }
}

#[test]
fn compatibility_barrier_freezes_until_matching_readiness() {
    let mut simulation = Simulation::new(config(ProgressionMode::CompatibilityBarrier)).unwrap();
    let first = simulation
        .step(TickInput::new(Tick::new(1)).with_commands(vec![request(1)]))
        .unwrap();
    assert_eq!(first.outcome, StepOutcome::Advanced);
    assert_eq!(
        simulation.pending_barrier(),
        Some(ResourceRequestId::new(1))
    );

    let waiting = simulation.step(TickInput::new(Tick::new(2))).unwrap();
    assert_eq!(waiting.outcome, StepOutcome::WaitingForReadiness);
    assert_eq!(waiting.tick, Tick::new(1));
    assert_eq!(waiting.state_digest, first.state_digest);

    let resumed = simulation
        .step(TickInput::new(Tick::new(2)).with_external_events(vec![ready(2, 1)]))
        .unwrap();
    assert_eq!(resumed.outcome, StepOutcome::Advanced);
    assert_eq!(simulation.pending_barrier(), None);
    assert_eq!(
        simulation.request_status(ResourceRequestId::new(1)),
        Some(ResourceRequestStatus::Ready)
    );
}

#[test]
fn continuing_world_consumes_the_same_readiness_tape_without_a_barrier() {
    let mut simulation = Simulation::new(config(ProgressionMode::ContinuingWorld)).unwrap();
    simulation
        .step(TickInput::new(Tick::new(1)).with_commands(vec![request(1)]))
        .unwrap();
    let result = simulation
        .step(TickInput::new(Tick::new(2)).with_external_events(vec![ready(2, 1)]))
        .unwrap();

    assert_eq!(result.outcome, StepOutcome::Advanced);
    assert_eq!(simulation.pending_barrier(), None);
    assert_eq!(
        simulation.request_status(ResourceRequestId::new(1)),
        Some(ResourceRequestStatus::Ready)
    );
}

#[test]
fn short_press_and_release_are_each_consumed_once() {
    let mut simulation = Simulation::new(config(ProgressionMode::ContinuingWorld)).unwrap();
    let result = simulation
        .step(TickInput::new(Tick::new(1)).with_commands(vec![
            button("confirm", ButtonTransition::Press),
            button("confirm", ButtonTransition::Release),
            button("confirm", ButtonTransition::Release),
        ]))
        .unwrap();

    assert_eq!(result.events.len(), 2);
    assert!(matches!(
        result.events[0].kind,
        DomainEventKind::ButtonPressed { .. }
    ));
    assert!(matches!(
        result.events[1].kind,
        DomainEventKind::ButtonReleased { .. }
    ));
    assert!(!simulation.button_held("confirm"));
    assert_eq!(result.events[0].trace_id, TraceId::new(1));
    assert_eq!(result.events[1].trace_id, TraceId::new(2));

    let next = simulation
        .step(TickInput::new(Tick::new(2)).with_commands(vec![
            button("confirm", ButtonTransition::Press),
            button("confirm", ButtonTransition::Press),
        ]))
        .unwrap();
    assert_eq!(next.events.len(), 1);
}

#[test]
fn batched_steps_emit_every_marker_and_completion_once_in_order() {
    let mut simulation = Simulation::new(config(ProgressionMode::ContinuingWorld)).unwrap();
    simulation
        .step(TickInput::new(Tick::new(1)).with_commands(vec![effect_command()]))
        .unwrap();

    let middle = simulation
        .step(TickInput::with_stride(Tick::new(5), 4))
        .unwrap();
    let markers: Vec<_> = middle
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            DomainEventKind::EffectMarker { marker, .. } => Some(marker.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        markers,
        [
            "fixture:effect/windup",
            "fixture:effect/first_hit",
            "fixture:effect/second_hit"
        ]
    );
    assert_eq!(middle.presentation.effects[0].elapsed_ticks, 4);

    let end = simulation
        .step(TickInput::with_stride(Tick::new(7), 2))
        .unwrap();
    assert!(matches!(
        &end.events[0].kind,
        DomainEventKind::EffectMarker { marker, .. } if marker.as_str() == "fixture:effect/final_hit"
    ));
    assert!(matches!(
        end.events[1].kind,
        DomainEventKind::EffectCompleted { .. }
    ));
    assert!(end.presentation.effects.is_empty());

    let after = simulation.step(TickInput::new(Tick::new(8))).unwrap();
    assert!(after.events.is_empty());
}

#[test]
fn invalid_timeline_rolls_back_every_command_in_the_step() {
    let mut simulation = Simulation::new(config(ProgressionMode::ContinuingWorld)).unwrap();
    let mut invalid = effect_command();
    if let GameCommand::StartEffect { timeline, .. } = &mut invalid {
        timeline.duration_ticks = 0;
    }

    let error = simulation
        .step(TickInput::new(Tick::new(1)).with_commands(vec![effect_command(), invalid]))
        .unwrap_err();
    assert!(error.context.contains("duration"));

    let next = simulation.step(TickInput::new(Tick::new(1))).unwrap();
    assert!(next.events.is_empty());
    assert!(next.presentation.effects.is_empty());
}

#[test]
fn stale_or_duplicate_completion_is_rejected_without_changing_state() {
    let mut simulation = Simulation::new(config(ProgressionMode::ContinuingWorld)).unwrap();
    simulation
        .step(TickInput::new(Tick::new(1)).with_commands(vec![request(1)]))
        .unwrap();
    let ready_result = simulation
        .step(TickInput::new(Tick::new(2)).with_external_events(vec![ready(2, 1)]))
        .unwrap();

    let error = simulation
        .step(TickInput::new(Tick::new(3)).with_external_events(vec![ready(3, 1)]))
        .unwrap_err();
    assert!(error.context.contains("stale"));
    assert_eq!(simulation.tick(), Tick::new(2));
    assert_eq!(
        simulation.state_digest().unwrap(),
        ready_result.state_digest.unwrap()
    );
}

#[test]
fn frame_stalls_leave_bounded_backlog_instead_of_dropping_ticks() {
    let mut bounded = PresentationClock::new(SimulationRate::per_second(60).unwrap(), 4).unwrap();
    let first = bounded.advance(Duration::from_millis(500)).unwrap();
    assert!(first.backlogged);
    assert_eq!(first.inputs.len(), 4);
    assert!(first.inputs.iter().all(|input| input.stride == 1));

    let mut total = first.inputs.iter().map(|input| input.stride).sum::<u32>();
    for _ in 0..20 {
        let drained = bounded.drain().unwrap();
        total += drained.inputs.iter().map(|input| input.stride).sum::<u32>();
        if !drained.backlogged {
            break;
        }
    }
    assert_eq!(total, 30);
    assert_eq!(bounded.tick(), Tick::new(30));
}

#[test]
fn pause_and_speed_changes_use_explicit_clock_policies() {
    let mut clock = PresentationClock::new(SimulationRate::per_second(60).unwrap(), 64).unwrap();
    let initial = clock.advance(Duration::from_millis(25)).unwrap();
    assert_eq!(
        initial.inputs.iter().map(|input| input.stride).sum::<u32>(),
        1
    );

    clock.set_paused(true);
    assert!(
        clock
            .advance(Duration::from_secs(10))
            .unwrap()
            .inputs
            .is_empty()
    );
    clock.set_paused(false);
    assert!(clock.drain().unwrap().inputs.is_empty());
    assert_eq!(
        clock
            .advance(Duration::from_millis(10))
            .unwrap()
            .inputs
            .len(),
        1
    );

    clock.set_speed_ppm(2_000_000).unwrap();
    let accelerated = clock.advance(Duration::from_millis(500)).unwrap();
    assert_eq!(
        accelerated
            .inputs
            .iter()
            .map(|input| input.stride)
            .sum::<u32>(),
        60
    );
    assert!(clock.set_speed_ppm(0).is_err());
}

#[test]
fn detailed_traces_are_bounded() {
    let mut simulation = Simulation::new(config(ProgressionMode::ContinuingWorld)).unwrap();
    for tick in 1..=600 {
        simulation
            .step(TickInput::new(Tick::new(tick)).with_commands(vec![
                button("confirm", ButtonTransition::Press),
                button("confirm", ButtonTransition::Release),
            ]))
            .unwrap();
    }
    assert_eq!(simulation.traces().count(), 1024);
}

#[test]
fn replay_envelope_preserves_initial_state_configuration_and_inputs() {
    let config = config(ProgressionMode::ContinuingWorld);
    let simulation = Simulation::new(config.clone()).unwrap();
    let tape = ReplayTape {
        schema_version: 1,
        initial_state_digest: simulation.state_digest().unwrap(),
        config,
        inputs: vec![
            TickInput::new(Tick::new(1)).with_commands(vec![request(1)]),
            TickInput::new(Tick::new(2)).with_external_events(vec![ready(2, 1)]),
        ],
    };

    let encoded = serde_json::to_string(&tape).unwrap();
    let restored: ReplayTape = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored, tape);
    assert_eq!(
        restored.initial_state_digest,
        simulation.state_digest().unwrap()
    );
    assert_eq!(restored.config.rng.algorithm, "none");
    assert_eq!(restored.inputs.len(), 2);
}

#[test]
fn diagnostic_policies_do_not_change_authoritative_outcomes() {
    let mut normal = Simulation::new(SimulationConfig {
        diagnostics: DiagnosticPolicy::default(),
        ..config(ProgressionMode::ContinuingWorld)
    })
    .unwrap();
    let mut verified = Simulation::new(config(ProgressionMode::ContinuingWorld)).unwrap();
    let mut normal_events = Vec::new();
    let mut verified_events = Vec::new();
    let mut normal_digest = None;
    let mut verified_digest = None;

    for tick in 1..=10 {
        let mut input = TickInput::new(Tick::new(tick));
        input.commands = commands_between(tick - 1, tick);
        input.external_events = events_between(tick - 1, tick);
        if tick == 10 {
            input = input.requesting_digest();
        }
        let normal_result = normal.step(input.clone()).unwrap();
        let verified_result = verified.step(input).unwrap();
        normal_events.extend(normal_result.events);
        verified_events.extend(verified_result.events);
        normal_digest = normal_result.state_digest;
        verified_digest = verified_result.state_digest;
        if tick < 10 {
            assert!(normal_result.presentation.tick.get() <= tick);
        }
    }

    assert_eq!(normal_events, verified_events);
    assert_eq!(normal_digest, verified_digest);
    assert!(normal_digest.is_some());
    assert_eq!(normal.traces().count(), 0);
    assert!(verified.traces().count() > 0);
}
