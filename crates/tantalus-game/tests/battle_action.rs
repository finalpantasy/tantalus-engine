use std::collections::VecDeque;
use tantalus_contracts::{ActionMarkerKind, ContentId, EngineError, EntityId, ErrorCategory, Tick};
use tantalus_game::battle::*;

fn id(text: &str) -> ContentId {
    ContentId::parse(text).unwrap()
}

fn action() -> ActionDefinition {
    ActionDefinition {
        id: id("fixture:attack/ruin"),
        power: 12,
        hit_rate: 90,
        mp_cost: 5,
        timeline: ActionTimeline {
            duration_ticks: 6,
            markers: vec![
                ActionMarker {
                    tick_offset: 1,
                    kind: ActionMarkerKind::Effect(id("fixture:effect/ruin")),
                },
                ActionMarker {
                    tick_offset: 2,
                    kind: ActionMarkerKind::Audio(id("fixture:audio/cast")),
                },
                ActionMarker {
                    tick_offset: 3,
                    kind: ActionMarkerKind::Hit,
                },
                ActionMarker {
                    tick_offset: 4,
                    kind: ActionMarkerKind::Hit,
                },
                ActionMarker {
                    tick_offset: 6,
                    kind: ActionMarkerKind::Complete,
                },
            ],
        },
    }
}

fn ruleset() -> CombatRuleset {
    let mut caster = CombatUnit::new(EntityId::new(1), Team::Player, 500, 20);
    caster.magic = 20;
    caster.level = 10;
    let mut enemy = CombatUnit::new(EntityId::new(2), Team::Enemy, 500, 0);
    enemy.magic_defense = 2;
    enemy.reward = RewardBundle {
        resolution_id: 0,
        experience: 50,
        gil: 20,
        items: vec![id("my_mod:weapon/raptor_talon")],
        shares: None,
    };
    CombatRuleset::new(vec![action()], vec![caster, enemy]).unwrap()
}

fn command() -> BattleCommand {
    BattleCommand {
        actor: EntityId::new(1),
        action: id("fixture:attack/ruin"),
        targets: vec![EntityId::new(2)],
    }
}

#[derive(Debug, Default)]
struct Services {
    rolls: VecDeque<u32>,
    committed: Vec<RewardBundle>,
    fail_commit: bool,
}

impl GameServices for Services {
    fn random_below(&mut self, _upper: u32) -> u32 {
        self.rolls.pop_front().unwrap_or(0)
    }

    fn commit_rewards(&mut self, rewards: &RewardBundle) -> Result<(), EngineError> {
        if self.fail_commit {
            return Err(EngineError::new(
                ErrorCategory::Internal,
                "synthetic reward failure",
                "fixture-services",
                true,
            ));
        }
        self.committed.push(rewards.clone());
        Ok(())
    }
}

fn advance(rules: &mut CombatRuleset, services: &mut Services, tick: u64) -> CombatUpdate {
    rules
        .advance(&CombatInput::new(Tick::new(tick)), services)
        .unwrap()
}

#[test]
fn complete_action_resolves_two_hits_and_commits_rewards_once() {
    let mut rules = ruleset();
    let mut services = Services::default();
    let mut all_events = Vec::new();

    let start = rules
        .advance(
            &CombatInput {
                command: Some(command()),
                ..CombatInput::new(Tick::new(1))
            },
            &mut services,
        )
        .unwrap();
    assert!(start.active_action);
    assert_eq!(rules.unit(EntityId::new(1)).unwrap().mp, 15);
    for tick in 2..=7 {
        all_events.extend(advance(&mut rules, &mut services, tick).events);
    }

    assert_eq!(rules.unit(EntityId::new(2)).unwrap().hp, 0);
    assert_eq!(rules.status(), BattleStatus::Victory);
    assert_eq!(services.committed.len(), 1);
    assert_eq!(services.committed[0].experience, 50);
    assert_eq!(services.committed[0].gil, 20);
    assert_eq!(
        services.committed[0].items,
        vec![id("my_mod:weapon/raptor_talon")]
    );
    assert_eq!(services.committed[0].resolution_id, 1);
    assert!(
        all_events
            .iter()
            .any(|event| matches!(event, CombatEvent::UnitDefeated { .. }))
    );

    advance(&mut rules, &mut services, 8);
    assert_eq!(services.committed.len(), 1);
}

#[test]
fn fast_forward_cannot_skip_hits_or_completion() {
    let mut rules = ruleset();
    let mut services = Services::default();
    rules
        .advance(
            &CombatInput {
                command: Some(command()),
                ..CombatInput::new(Tick::new(1))
            },
            &mut services,
        )
        .unwrap();

    let result = rules
        .advance(
            &CombatInput {
                stride: 6,
                ..CombatInput::new(Tick::new(7))
            },
            &mut services,
        )
        .unwrap();

    assert_eq!(result.markers.len(), 5);
    assert_eq!(result.outcome, Some(BattleOutcome::Victory));
    assert_eq!(rules.unit(EntityId::new(2)).unwrap().hp, 0);
    assert_eq!(services.committed.len(), 1);
}

#[test]
fn interruption_cancels_without_damage_or_rewards() {
    let mut rules = ruleset();
    let mut services = Services::default();
    rules
        .advance(
            &CombatInput {
                command: Some(command()),
                ..CombatInput::new(Tick::new(1))
            },
            &mut services,
        )
        .unwrap();

    let interrupted = rules
        .advance(
            &CombatInput {
                interrupt: Some(EntityId::new(1)),
                ..CombatInput::new(Tick::new(2))
            },
            &mut services,
        )
        .unwrap();

    assert!(
        interrupted
            .events
            .iter()
            .any(|event| matches!(event, CombatEvent::ActionInterrupted { .. }))
    );
    assert!(!interrupted.active_action);
    assert_eq!(rules.unit(EntityId::new(2)).unwrap().hp, 500);
    assert!(services.committed.is_empty());
}

#[test]
fn missed_hit_does_not_create_a_victory() {
    let mut rules = ruleset();
    rules.unit_mut(EntityId::new(1)).unwrap().level = 1;
    let mut services = Services {
        rolls: VecDeque::from([95, 95]),
        ..Services::default()
    };
    rules
        .advance(
            &CombatInput {
                command: Some(command()),
                ..CombatInput::new(Tick::new(1))
            },
            &mut services,
        )
        .unwrap();
    for tick in 2..=7 {
        advance(&mut rules, &mut services, tick);
    }

    assert_eq!(
        rules.status(),
        BattleStatus::Running,
        "rules: {rules:?}; services: {services:?}"
    );
    assert_eq!(rules.unit(EntityId::new(2)).unwrap().hp, 500);
    assert!(services.committed.is_empty());
}

#[test]
fn failed_reward_commit_rolls_back_victory_state_for_retry() {
    let mut rules = ruleset();
    let mut services = Services {
        fail_commit: true,
        ..Services::default()
    };
    rules
        .advance(
            &CombatInput {
                command: Some(command()),
                ..CombatInput::new(Tick::new(1))
            },
            &mut services,
        )
        .unwrap();
    for tick in 2..=6 {
        advance(&mut rules, &mut services, tick);
    }

    let error = rules
        .advance(&CombatInput::new(Tick::new(7)), &mut services)
        .unwrap_err();
    assert!(error.context.contains("reward failure"));
    assert_eq!(rules.status(), BattleStatus::Running);

    services.fail_commit = false;
    let retry = rules
        .advance(&CombatInput::new(Tick::new(7)), &mut services)
        .unwrap();
    assert_eq!(retry.outcome, Some(BattleOutcome::Victory));
    assert_eq!(services.committed.len(), 1);
}

#[test]
fn invalid_command_does_not_spend_mp_or_start_an_action() {
    let mut rules = ruleset();
    let mut services = Services::default();
    let mut invalid = command();
    invalid.targets.push(EntityId::new(2));

    let error = rules
        .advance(
            &CombatInput {
                command: Some(invalid),
                ..CombatInput::new(Tick::new(1))
            },
            &mut services,
        )
        .unwrap_err();

    assert!(error.context.contains("duplicate action target"));
    assert_eq!(rules.unit(EntityId::new(1)).unwrap().mp, 20);
    assert_eq!(rules.tick(), Tick::default());
}

#[test]
fn defeat_has_no_victory_rewards() {
    let mut rules = ruleset();
    rules.unit_mut(EntityId::new(1)).unwrap().hp = 1;
    rules.unit_mut(EntityId::new(2)).unwrap().mp = 5;
    let mut services = Services::default();
    let enemy_command = BattleCommand {
        actor: EntityId::new(2),
        action: id("fixture:attack/ruin"),
        targets: vec![EntityId::new(1)],
    };
    rules
        .advance(
            &CombatInput {
                command: Some(enemy_command),
                ..CombatInput::new(Tick::new(1))
            },
            &mut services,
        )
        .unwrap();
    for tick in 2..=7 {
        advance(&mut rules, &mut services, tick);
    }

    assert_eq!(rules.status(), BattleStatus::Defeat);
    assert!(!rules.unit(EntityId::new(1)).unwrap().alive);
    assert!(services.committed.is_empty());
}

#[test]
fn action_validation_rejects_missing_or_unordered_completion() {
    let mut invalid = action();
    invalid.timeline.markers.pop();
    assert!(CombatRuleset::new(vec![invalid], Vec::new()).is_err());

    let mut unordered = action();
    unordered.timeline.markers.reverse();
    assert!(CombatRuleset::new(vec![unordered], Vec::new()).is_err());
}
