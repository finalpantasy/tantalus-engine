//! B6 battle AI: synthetic programs whose expected picks are derived by hand
//! from the Memoria source (`RequestAction`, `Request`, `BTLCMD`, `GetSysvar`,
//! `GetBattleData`, `GetSysList`).

use std::collections::BTreeMap;
use tantalus_contracts::EntityId;
use tantalus_legacy::battle::host::{BattleDataView, CurrentCommandView, UnitView};
use tantalus_legacy::battle::rng::{Draw, SequenceRandom};
use tantalus_legacy::battle::script_host::{
    AiEvent, BattleScriptDriver, BattleScriptState, EnemyCommand, EnemyCommandKind, ReactionKind,
    ScriptEntry, ScriptTable, character_entry,
};
use tantalus_legacy::{
    EventProgram, Expression, ExpressionOp as Op, Instruction, InstructionKind as Kind,
    VariableRef, VariableScope, VariableType,
};

const ENEMY: u16 = 16; // slot 4

fn program(kinds: Vec<Kind>) -> EventProgram {
    let mut instructions = BTreeMap::new();
    for (offset, kind) in kinds.into_iter().enumerate() {
        instructions.insert(
            offset as u32,
            Instruction {
                offset: offset as u32,
                length: 1,
                kind,
            },
        );
    }
    EventProgram {
        instructions,
        source_length: 0,
    }
}

fn expr(ops: Vec<Op>) -> Expression {
    let mut ops = ops;
    ops.push(Op::End);
    Expression(ops)
}

fn local(value_type: VariableType, index: u16) -> Op {
    Op::Variable(VariableRef {
        scope: VariableScope::Instance,
        value_type,
        index,
    })
}

fn global(value_type: VariableType, index: u16) -> Op {
    Op::Variable(VariableRef {
        scope: VariableScope::Global,
        value_type,
        index,
    })
}

fn entry(variable_count: u8, functions: Vec<(u16, Vec<Kind>)>) -> ScriptEntry {
    ScriptEntry {
        variable_count,
        functions: functions
            .into_iter()
            .map(|(tag, kinds)| (tag, program(kinds)))
            .collect(),
    }
}

/// Entry 0 spawns entry 1 as the first enemy; entry 1 carries `functions`.
fn table(functions: Vec<(u16, Vec<Kind>)>) -> ScriptTable {
    ScriptTable {
        entries: vec![
            entry(
                0,
                vec![(
                    0,
                    vec![
                        Kind::InitializeObject {
                            source: 1,
                            unique_id: 128,
                        },
                        Kind::Return,
                    ],
                )],
            ),
            entry(8, functions),
        ],
    }
}

fn unit(hp: i32) -> UnitView {
    UnitView {
        hp,
        max_hp: hp,
        ..UnitView::default()
    }
}

fn state() -> BattleScriptState {
    let data = BattleDataView {
        battle_map_no: 100,
        player_id_mask: 0x0F,
        enemy_id_mask: u32::from(ENEMY),
        all_id_mask: 0x1F,
        ..BattleDataView::default()
    };
    let mut units = BTreeMap::new();
    for (bit, hp) in [(1u64, 400), (2, 300), (4, 200), (8, 100), (16, 5000)] {
        units.insert(EntityId::new(bit), unit(hp));
    }
    BattleScriptState::new(data, units)
}

fn rolls(values: &[i32]) -> Box<SequenceRandom> {
    let kinds: Vec<(Draw, i32)> = values.iter().map(|v| (Draw::Random8, *v)).collect();
    Box::new(SequenceRandom::with_kinds(&kinds))
}

fn driver(
    functions: Vec<(u16, Vec<Kind>)>,
    state: BattleScriptState,
    draws: &[i32],
) -> BattleScriptDriver {
    BattleScriptDriver::new(table(functions), state, rolls(draws), &[])
}

fn attack_by_index(modulus: i32) -> Vec<Kind> {
    vec![
        Kind::Evaluate(expr(vec![Op::SystemList(0), Op::Constant(1), Op::Assign])),
        Kind::Evaluate(expr(vec![
            local(VariableType::Byte, 0),
            Op::SystemVariable(0),
            Op::Constant(modulus),
            Op::Remainder,
            Op::Assign,
        ])),
        Kind::QueueEnemyAttack {
            attack: expr(vec![local(VariableType::Byte, 0)]),
        },
        Kind::Return,
    ]
}

fn idle_loop() -> Vec<Kind> {
    // Wait(1); Jump back to the wait (relative to the next instruction).
    vec![Kind::Wait(expr(vec![Op::Constant(1)])), Kind::Jump(-2)]
}

fn boot(driver: &mut BattleScriptDriver) {
    // tick 1: objects New -> Init; tick 2: main runs InitializeObject and the
    // enemy appears; tick 3: the enemy's tag 0 returns and level becomes 7.
    for _ in 0..4 {
        driver.tick().unwrap();
    }
}

#[test]
fn enemy_atb_pick_follows_random_and_modulus() {
    // draws 200, 7, 9 with `% 3` give 200 % 3 = 2, 7 % 3 = 1, 9 % 3 = 0.
    let mut ai = driver(
        vec![
            (0, vec![Kind::Return]),
            (1, idle_loop()),
            (5, attack_by_index(3)),
        ],
        state(),
        &[200, 7, 9],
    );
    // Two ticks in, the enemy object exists but is still in its init
    // function at level 0: the request is refused (`p.level > 3` fails).
    ai.tick().unwrap();
    ai.tick().unwrap();
    assert_eq!(ai.object_level(4), Some(0));
    assert!(!ai.request_atb(ENEMY));
    ai.tick().unwrap();
    ai.tick().unwrap();
    assert_eq!(ai.object_level(4), Some(7));
    let mut picks = Vec::new();
    for _ in 0..3 {
        assert!(ai.request_atb(ENEMY));
        let command = ai.run_until_attack(ENEMY, 8).unwrap().expect("attack");
        picks.push((command.attack, command.target_mask, command.kind));
        // The ATB function returned: the object is idle at level 7 again.
        assert_eq!(ai.object_level(4), Some(7));
    }
    assert_eq!(
        picks,
        vec![
            (2, 1, EnemyCommandKind::Attack),
            (1, 1, EnemyCommandKind::Attack),
            (0, 1, EnemyCommandKind::Attack),
        ]
    );
}

#[test]
fn atb_request_is_refused_while_the_function_runs() {
    // tag 5 waits two frames before queueing: a second request in between
    // fails because the object is at level 3 (`Request` needs level < obj.level).
    let mut wait_then_attack = vec![Kind::Wait(expr(vec![Op::Constant(2)]))];
    wait_then_attack.extend(attack_by_index(3));
    let mut ai = driver(
        vec![(0, vec![Kind::Return]), (5, wait_then_attack)],
        state(),
        &[5],
    );
    boot(&mut ai);
    assert!(ai.request_atb(ENEMY));
    ai.tick().unwrap(); // end of tick: btlchk loop starts tag 5
    assert_eq!(ai.object_level(4), Some(3));
    assert!(!ai.request_atb(ENEMY));
    let command = ai.run_until_attack(ENEMY, 8).unwrap().expect("attack");
    assert_eq!(command.attack, 5 % 3);
}

#[test]
fn counter_reads_the_triggering_command_and_target() {
    // tag 6 (level 1): attack = sysvar 29 + sysvar 28; sysvars 28/29 read
    // `_btlCmdPrmCmd/_btlCmdPrmSub` at level <= 2. `SetSysList(0, reactCaster)`.
    let counter = vec![
        Kind::QueueEnemyAttack {
            attack: expr(vec![
                Op::SystemVariable(29),
                Op::SystemVariable(28),
                Op::Add,
            ]),
        },
        Kind::Return,
    ];
    let mut ai = driver(vec![(0, vec![Kind::Return]), (6, counter)], state(), &[]);
    boot(&mut ai);
    assert!(ai.request_reaction(ReactionKind::Counter, ENEMY, 2, 25, 103));
    ai.tick().unwrap();
    let events = ai.state.events.clone();
    assert_eq!(
        events,
        vec![AiEvent::Command(EnemyCommand {
            caster: ENEMY,
            kind: EnemyCommandKind::Counter,
            attack: 128,
            target_mask: 2,
        })]
    );
    assert_eq!(ai.object_level(4), Some(7));
}

#[test]
fn dying_runs_at_level_zero_and_reaction_queues_nothing() {
    let attack_one = vec![
        Kind::QueueEnemyAttack {
            attack: expr(vec![Op::Constant(77)]),
        },
        Kind::Return,
    ];
    let mut ai = driver(
        vec![
            (0, vec![Kind::Return]),
            (7, attack_one.clone()),
            (9, attack_one),
        ],
        state(),
        &[],
    );
    boot(&mut ai);
    assert!(ai.request_reaction(ReactionKind::Reaction, ENEMY, 1, 25, 103));
    ai.tick().unwrap();
    assert!(ai.request_reaction(ReactionKind::Dying, ENEMY, 4, 25, 103));
    ai.tick().unwrap();
    assert_eq!(
        ai.state.events,
        vec![
            // `BTLCMD` has no case for level 2.
            AiEvent::AttackIgnored {
                caster: ENEMY,
                level: 2,
                attack: 77
            },
            AiEvent::Command(EnemyCommand {
                caster: ENEMY,
                kind: EnemyCommandKind::Dying,
                attack: 77,
                target_mask: 4,
            }),
        ]
    );
}

#[test]
fn counter_interrupts_a_running_atb_function_and_it_resumes() {
    // ATB function: Wait(3) then attack 9. A counter (level 1 < 3) is accepted
    // mid-wait, runs, and the ATB function continues afterwards.
    let atb = vec![
        Kind::Wait(expr(vec![Op::Constant(3)])),
        Kind::QueueEnemyAttack {
            attack: expr(vec![Op::Constant(9)]),
        },
        Kind::Return,
    ];
    let counter = vec![
        Kind::QueueEnemyAttack {
            attack: expr(vec![Op::Constant(4)]),
        },
        Kind::Return,
    ];
    let mut ai = driver(
        vec![(0, vec![Kind::Return]), (5, atb), (6, counter)],
        state(),
        &[],
    );
    boot(&mut ai);
    assert!(ai.request_atb(ENEMY));
    ai.tick().unwrap();
    assert_eq!(ai.object_level(4), Some(3));
    assert!(ai.request_reaction(ReactionKind::Counter, ENEMY, 1, 25, 103));
    assert_eq!(ai.object_level(4), Some(1));
    let command = ai.run_until_attack(ENEMY, 12).unwrap().expect("attack");
    assert_eq!(command.attack, 9);
    assert_eq!(command.kind, EnemyCommandKind::Attack);
    let kinds: Vec<_> = ai
        .state
        .events
        .iter()
        .filter_map(|e| match e {
            AiEvent::Command(c) => Some((c.kind, c.attack)),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            (EnemyCommandKind::Counter, 4),
            (EnemyCommandKind::Attack, 9)
        ]
    );
}

#[test]
fn atb_function_reads_current_command_and_scene_sysvars() {
    // attack = sysvar 29 (sub_no 9) + sysvar 24 (map 100) + sysvar 37 (tar 6).
    let atb = vec![
        Kind::QueueEnemyAttack {
            attack: expr(vec![
                Op::SystemVariable(29),
                Op::SystemVariable(24),
                Op::Add,
                Op::SystemVariable(37),
                Op::Add,
            ]),
        },
        Kind::Return,
    ];
    let mut s = state();
    s.data.current_command = Some(CurrentCommandView {
        cmd_no: 7,
        sub_no: 9,
        tar_id: 6,
        ..CurrentCommandView::default()
    });
    let mut ai = driver(vec![(0, vec![Kind::Return]), (5, atb)], s, &[]);
    boot(&mut ai);
    assert!(ai.request_atb(ENEMY));
    let command = ai.run_until_attack(ENEMY, 8).unwrap().expect("attack");
    // Level 3 reads the current command, not the request parameters.
    assert_eq!(command.attack, 9 + 100 + 6);
}

#[test]
fn auto_attack_status_suppresses_the_atb_attack() {
    let mut s = state();
    s.auto_attack_mask = ENEMY;
    let mut ai = driver(
        vec![(0, vec![Kind::Return]), (5, attack_by_index(4))],
        s,
        &[9],
    );
    boot(&mut ai);
    assert!(ai.request_atb(ENEMY));
    ai.run_until_attack(ENEMY, 6).unwrap();
    assert_eq!(
        ai.state.events,
        vec![AiEvent::AttackSuppressed {
            caster: ENEMY,
            attack: 1
        }]
    );
    assert!(ai.take_attack(ENEMY).is_none());
}

#[test]
fn battle_codes_message_and_terminate_are_recorded_and_state_applies() {
    // RunBattleCode(38, 500) adds gil; (44, 70) sets AP; sysvar 6 reads gil.
    let atb = vec![
        Kind::RunBattleCode {
            code: expr(vec![Op::Constant(38)]),
            value: expr(vec![Op::Constant(500)]),
        },
        Kind::RunBattleCode {
            code: expr(vec![Op::Constant(44)]),
            value: expr(vec![Op::Constant(70)]),
        },
        Kind::ShowBattleMessage(expr(vec![Op::Constant(12)])),
        Kind::Evaluate(expr(vec![
            global(VariableType::Int24, 0),
            Op::SystemVariable(6),
            Op::Assign,
        ])),
        Kind::TerminateBattle,
        Kind::Return,
    ];
    let mut s = state();
    s.data.gil = 100;
    let mut ai = driver(vec![(0, vec![Kind::Return]), (5, atb)], s, &[]);
    boot(&mut ai);
    assert!(ai.request_atb(ENEMY));
    for _ in 0..3 {
        ai.tick().unwrap();
    }
    assert_eq!(
        ai.state.events,
        vec![
            AiEvent::BattleCode {
                code: 38,
                value: 500
            },
            AiEvent::BattleCode {
                code: 44,
                value: 70
            },
            AiEvent::Message { text: 12 },
            AiEvent::TerminateBattle,
        ]
    );
    assert_eq!(ai.state.data.gil, 600);
    assert_eq!(ai.state.data.bonus_ap, 70);
    // The script read sysvar 6 after the add: shared Global memory holds 600.
    let value = ai
        .state
        .global
        .as_ref()
        .unwrap()
        .get(0, VariableType::Int24)
        .unwrap();
    assert_eq!(value, 600);
}

#[test]
fn special_attack_records_caster_and_target() {
    let atb = vec![
        Kind::Evaluate(expr(vec![Op::SystemList(0), Op::Constant(4), Op::Assign])),
        Kind::QueueSpecialAttack {
            attack: expr(vec![Op::Constant(33)]),
        },
        Kind::Return,
    ];
    let mut ai = driver(vec![(0, vec![Kind::Return]), (5, atb)], state(), &[]);
    boot(&mut ai);
    assert!(ai.request_atb(ENEMY));
    for _ in 0..3 {
        ai.tick().unwrap();
    }
    assert_eq!(
        ai.state.events,
        vec![AiEvent::SpecialAttack {
            caster: ENEMY,
            target_mask: 4,
            attack: 33
        }]
    );
}

#[test]
fn instance_variables_persist_across_functions_and_syslist_masks_read() {
    // tag 0 stores 41 into an instance byte; tag 5 attacks with that byte + 1,
    // and SysList(1) is the object's own bit, SysList(2) the player mask.
    let init = vec![
        Kind::Evaluate(expr(vec![
            local(VariableType::Byte, 3),
            Op::Constant(41),
            Op::Assign,
        ])),
        Kind::Return,
    ];
    let atb = vec![
        Kind::QueueEnemyAttack {
            attack: expr(vec![
                local(VariableType::Byte, 3),
                Op::Constant(1),
                Op::Add,
                Op::SystemList(1),
                Op::Add,
                Op::SystemList(2),
                Op::Add,
            ]),
        },
        Kind::Return,
    ];
    let mut ai = driver(vec![(0, init), (5, atb)], state(), &[]);
    boot(&mut ai);
    assert!(ai.request_atb(ENEMY));
    let command = ai.run_until_attack(ENEMY, 8).unwrap().expect("attack");
    assert_eq!(command.attack, 42 + 16 + 15);
}

#[test]
fn member_reads_use_the_selected_object_slot() {
    // attack = HP of the first object in SysList(1) (the enemy, 5000 HP),
    // read through member 36 (`GetCharacterData` max/current HP table).
    let atb = vec![
        Kind::Evaluate(expr(vec![
            local(VariableType::UInt16, 0),
            Op::SystemList(1),
            Op::Member(36),
            Op::PickFirstMember,
            Op::Assign,
        ])),
        Kind::QueueEnemyAttack {
            attack: expr(vec![local(VariableType::UInt16, 0)]),
        },
        Kind::Return,
    ];
    let mut ai = driver(vec![(0, vec![Kind::Return]), (5, atb)], state(), &[]);
    boot(&mut ai);
    assert!(ai.request_atb(ENEMY));
    let command = ai.run_until_attack(ENEMY, 8).unwrap().expect("attack");
    assert_eq!(command.attack, 5000);
}

#[test]
fn party_actors_fill_the_low_slots_from_the_character_entries() {
    // 11 entries: entry 2 is Zidane (link 0), entry 3 Vivi (1); the vanilla
    // link rule is `index + 9 - count`, and the last entry is Beatrix.
    assert_eq!(character_entry(11, 0), Some(2));
    assert_eq!(character_entry(11, 1), Some(3));
    assert_eq!(character_entry(11, 7), Some(9));
    assert_eq!(character_entry(11, 11), Some(10));
    assert_eq!(character_entry(11, 9), None);
    assert_eq!(character_entry(2, 7), Some(0));

    let mut entries = table(vec![(0, vec![Kind::Return])]).entries;
    while entries.len() < 11 {
        entries.push(entry(0, vec![(0, vec![Kind::Return])]));
    }
    let ai = BattleScriptDriver::new(
        ScriptTable { entries },
        state(),
        rolls(&[]),
        &[1, 0], // Vivi, Zidane in party order
    );
    // Object slots follow party order: slot 0 = Vivi's entry, slot 1 = Zidane's.
    assert_eq!(ai.object_entry(0), Some(3));
    assert_eq!(ai.object_entry(1), Some(2));
    assert_eq!(ai.object_entry(2), None);
}

// ---------------------------------------------------------------------------
// Machine wiring
// ---------------------------------------------------------------------------

use tantalus_legacy::battle::machine::{Battle, BattleConfig, BattleEvent, BattleHooks};
use tantalus_legacy::battle::queue::{CommandSpec, cmd};
use tantalus_legacy::battle::script_host::{EnemyAiHooks, SceneOverlay, sync_state_from_battle};
use tantalus_legacy::battle::targeting::target_type;
use tantalus_legacy::battle::{BattleCommand, BattleUnit, StatusDataRow, StatusTable};

struct Catalog;

impl BattleHooks for Catalog {
    fn spec(&mut self, cmd_no: u32, _sub: i32, _c: &BattleUnit) -> Option<CommandSpec> {
        (cmd_no == cmd::ENEMY_ATK).then(|| {
            let mut command = BattleCommand::new(1);
            command.target_type = target_type::SINGLE_ENEMY;
            CommandSpec {
                command,
                default_ally: false,
                for_dead: false,
            }
        })
    }
}

fn machine_unit(id: u16, player: bool) -> BattleUnit {
    let mut u = BattleUnit::new(id, player);
    u.level = 10;
    u.dexterity = 40;
    u.cur_hp = 500;
    u.max_hp = 500;
    u
}

#[test]
fn full_gauge_enemy_gets_its_script_pick_through_the_machine_hook() {
    let rows = (0..32)
        .map(|_| StatusDataRow {
            opr_cnt: 0,
            conti_cnt: 0,
            clear_on_apply: 0,
            immunity_provided: 0,
        })
        .collect();
    let mut battle = Battle::new(
        vec![machine_unit(1, true), machine_unit(ENEMY, false)],
        BattleConfig::default(),
        StatusTable::new(rows),
    );
    let ai = driver(
        vec![
            (0, vec![Kind::Return]),
            (1, idle_loop()),
            (5, attack_by_index(3)),
        ],
        state(),
        &[200],
    );
    let mut hooks = EnemyAiHooks::new(Catalog, ai, SceneOverlay::default());
    sync_state_from_battle(&mut hooks.driver.state, &battle, &hooks.overlay);
    for _ in 0..4 {
        hooks.driver.tick().unwrap();
    }
    let mut machine_rng = SequenceRandom::r16(&[]);
    for _ in 0..200 {
        battle.tick(&mut hooks, &mut machine_rng).expect("tick");
        if battle.log.iter().any(|(_, e)| {
            matches!(
                e,
                BattleEvent::CommandQueued {
                    cmd_no: cmd::ENEMY_ATK,
                    ..
                }
            )
        }) {
            break;
        }
    }
    assert!(hooks.error.is_none(), "{:?}", hooks.error);
    // Gauge max 3200 at dex 40, +40 per tick: full on tick 80. The ATB request
    // lands on that tick, the function starts the tick after and queues on
    // the one after that; 200 % 3 = 2 is the enemy attack index.
    let queued: Vec<_> = battle
        .log
        .iter()
        .filter_map(|(tick, e)| match e {
            BattleEvent::CommandQueued {
                cmd_no: cmd::ENEMY_ATK,
                tar_id,
                unit,
            } => Some((*tick, *unit, *tar_id)),
            _ => None,
        })
        .collect();
    assert_eq!(queued, vec![(82, 1, 1)]);
    assert_eq!(battle.cmds.slots[battle.cmds.queue[0]].sub_no, 2);
}

/// `TimerUI` keeps running through a fight: the Festival of the Hunt scripts
/// read system variables 21 (`TimerUI.Enable`) and 17 (`TimerUI.Time`) and end
/// the battle when the clock reaches 0.
#[test]
fn battle_scripts_read_the_running_field_clock() {
    use tantalus_legacy::battle::script_host::BattleTimer;
    let rows = (0..32)
        .map(|_| StatusDataRow {
            opr_cnt: 0,
            conti_cnt: 0,
            clear_on_apply: 0,
            immunity_provided: 0,
        })
        .collect();
    let mut battle = Battle::new(
        vec![machine_unit(1, true), machine_unit(ENEMY, false)],
        BattleConfig::default(),
        StatusTable::new(rows),
    );
    let mut script = state();
    sync_state_from_battle(&mut script, &battle, &SceneOverlay::default());
    assert!(!script.data.timer_shown);
    let overlay = SceneOverlay {
        timer: Some(BattleTimer {
            time: 2.4,
            running: true,
        }),
        ..SceneOverlay::default()
    };
    sync_state_from_battle(&mut script, &battle, &overlay);
    assert!(script.data.timer_shown);
    assert_eq!(script.data.timer_seconds, 2);
    // 15 battle ticks are one second.
    battle.tick_no = 15;
    sync_state_from_battle(&mut script, &battle, &overlay);
    assert_eq!(script.data.timer_seconds, 1);
    battle.tick_no = 60;
    sync_state_from_battle(&mut script, &battle, &overlay);
    assert_eq!(script.data.timer_seconds, 0);
    // A paused clock (`TimerUI.Play` off) stays where it was.
    let paused = SceneOverlay {
        timer: Some(BattleTimer {
            time: 2.4,
            running: false,
        }),
        ..SceneOverlay::default()
    };
    sync_state_from_battle(&mut script, &battle, &paused);
    assert_eq!(script.data.timer_seconds, 2);
}

/// `BTLSEQ` through the hooks: the main loop's `QueueSpecialAttack` reaches the machine as a
/// script sequence (`btlseq.StartBtlSeq`) during the event phase, and the next frame's
/// scripts read `IsAttacking` (sysvar 25, `BtlSeqBusy`) as 1, so a script waiting on it
/// (BU_R004's opening, the Ragtime Mouse quiz) goes on.
#[test]
fn a_script_special_attack_starts_a_sequence_the_scripts_see_as_busy() {
    let rows = (0..32)
        .map(|_| StatusDataRow {
            opr_cnt: 0,
            conti_cnt: 0,
            clear_on_apply: 0,
            immunity_provided: 0,
        })
        .collect();
    let mut config = BattleConfig::default();
    config.timing.effect_after_loop_ticks = 20;
    config.timing.loop_total_ticks = 40;
    let units = [machine_unit(1, true), machine_unit(ENEMY, false)].map(|mut u| {
        u.will = 10;
        u
    });
    let mut battle = Battle::new(units.to_vec(), config, StatusTable::new(rows));
    battle.begin_special_start();
    let mut main = vec![
        Kind::Evaluate(expr(vec![Op::SystemList(0), Op::Constant(1), Op::Assign])),
        Kind::QueueSpecialAttack {
            attack: expr(vec![Op::Constant(0)]),
        },
    ];
    main.extend(idle_loop());
    let ai = driver(vec![(0, vec![Kind::Return]), (1, main)], state(), &[]);
    let mut hooks = EnemyAiHooks::new(Catalog, ai, SceneOverlay::default());
    let mut machine_rng = SequenceRandom::r16(&[0; 16]);
    let mut started = None;
    let mut busy_seen = false;
    for tick in 0..30 {
        battle.tick(&mut hooks, &mut machine_rng).expect("tick");
        if started.is_none() && !battle.event_sequences.is_empty() {
            started = Some(tick);
        }
        busy_seen |= hooks.driver.state.data.seq_busy;
    }
    assert!(hooks.error.is_none(), "{:?}", hooks.error);
    assert!(started.is_some(), "no sequence started");
    assert!(busy_seen, "the scripts never read IsAttacking as 1");
    assert!(battle.cmds.queue.is_empty() && battle.running.is_empty());
}
