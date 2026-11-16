//! Battle script host (G5 first slice): sysvar/system-list reads answer
//! `EventEngine.GetSysvar` / `btl_scrp.GetBattleData` / `GetSysList`
//! semantics against a `BattleDataView`, and a decoded script reads them
//! through the VM.

use tantalus_contracts::Tick;
use tantalus_legacy::battle::host::{BattleDataView, BattleScriptHost, CurrentCommandView};
use tantalus_legacy::battle::rng::{Draw, SequenceRandom};
use tantalus_legacy::{EventVm, LegacyEventHost, decode_program};

fn evaluate(expression: Vec<u8>) -> Vec<u8> {
    let mut bytes = vec![5];
    bytes.extend(expression);
    bytes.push(127);
    bytes
}

fn sysvar(index: u8) -> Vec<u8> {
    vec![122, index]
}

fn syslist(index: u8) -> Vec<u8> {
    vec![121, index]
}

fn global_int24(index: u16) -> Vec<u8> {
    vec![0xc0 | (2 << 2), index as u8]
}

fn sample_data() -> BattleDataView {
    BattleDataView {
        battle_map_no: 0x55,
        seq_busy: true,
        result_mode: false,
        battle_result: 1,
        btl_phase: 4,
        pattern_id: 7,
        start_type: 0,
        gil: 1234,
        btl_seq: 2,
        load_status: 1,
        bonus_exp: 640,
        phoenix_used: true,
        bonus_ap: 12,
        current_command: Some(CurrentCommandView {
            cmd_no: 3,
            sub_no: 0x1D,
            regist_btl_id: 0x10,
            tar_id: 0x03,
            weapon_element: 5,
            cmd_element: 2,
            effect_counter: 2,
        }),
        player_id_mask: 0x0F,
        enemy_id_mask: 0x30,
        all_id_mask: 0x3F,
    }
}

#[test]
fn battle_sysvars_answer_getsysvar_and_getbattledata() {
    let data = sample_data();
    let mut host = BattleScriptHost::new(&data);
    let read = |host: &mut BattleScriptHost, code: u8| {
        LegacyEventHost::read_system_variable(host, code).unwrap()
    };
    assert_eq!(read(&mut host, 24), 0x55);
    assert_eq!(read(&mut host, 25), 1);
    assert_eq!(read(&mut host, 26), 0);
    assert_eq!(read(&mut host, 27), 1);
    assert_eq!(read(&mut host, 28), 3);
    assert_eq!(read(&mut host, 29), 0x1D);
    assert_eq!(read(&mut host, 30), 4);
    assert_eq!(read(&mut host, 31), 7);
    assert_eq!(read(&mut host, 32), 0);
    assert_eq!(read(&mut host, 33), 1234, "gil via GetBattleData");
    assert_eq!(read(&mut host, 34), 4, "btl_phase");
    assert_eq!(read(&mut host, 35), 2);
    assert_eq!(read(&mut host, 36), 0x10);
    assert_eq!(read(&mut host, 37), 3);
    assert_eq!(read(&mut host, 38), 5);
    assert_eq!(read(&mut host, 39), 2);
    assert_eq!(read(&mut host, 40), 1);
    assert_eq!(read(&mut host, 41), 640, "bonus exp");
    assert_eq!(read(&mut host, 42), 1, "phoenix flag");
    assert_eq!(read(&mut host, 43), 2, "effect counter");
    assert_eq!(read(&mut host, 44), 12, "bonus ap");
    // `GetBattleData` default for uncovered codes below 192.
    assert_eq!(read(&mut host, 45), -1);
    assert_eq!(read(&mut host, 100), -1);
    // Field-context sysvars are the documented zero deviation.
    assert_eq!(read(&mut host, 2), 0);
    assert_eq!(read(&mut host, 191), 0);
}

#[test]
fn command_sysvars_default_to_minus_one_without_a_command() {
    let mut data = sample_data();
    data.current_command = None;
    let mut host = BattleScriptHost::new(&data);
    let read = |host: &mut BattleScriptHost, code: u8| {
        LegacyEventHost::read_system_variable(host, code).unwrap()
    };
    assert_eq!(read(&mut host, 28), 0, "cmd_no falls back to none");
    assert_eq!(read(&mut host, 29), 0);
    assert_eq!(read(&mut host, 36), -1, "GetBattleData default");
    assert_eq!(read(&mut host, 43), -1);
}

#[test]
fn system_list_masks_and_live_unit_masks() {
    let data = sample_data();
    let mut host = BattleScriptHost::new(&data);
    let read = |host: &mut BattleScriptHost, index: u8| {
        LegacyEventHost::read_system_list(host, index).unwrap()
    };
    let write = |host: &mut BattleScriptHost, index: u8, value: i32| {
        LegacyEventHost::write_system_list(host, index, value).unwrap()
    };
    assert_eq!(read(&mut host, 2), 0x0F, "player mask");
    assert_eq!(read(&mut host, 3), 0x30, "enemy mask");
    assert_eq!(read(&mut host, 4), 0x3F, "all mask");
    // Element numbers mask with 7.
    write(&mut host, 9, 0x1ABCD);
    assert_eq!(read(&mut host, 1), 0xABCD, "stores truncate to 16 bits");
    assert_eq!(read(&mut host, 9), 0xABCD, "reads mask the index too");
    // Element 6 recomputes from element 5; command stat access is not
    // modeled and reads back 0.
    write(&mut host, 5, 43);
    assert_eq!(read(&mut host, 6), 0);
}

#[test]
fn decoded_script_reads_battle_sysvars_through_the_vm() {
    // Two sysvar reads stored to globals: bonus exp (41) and the enemy
    // mask (system-list element 3).
    let result_exp = global_int24(31);
    let result_mask = global_int24(32);
    let program = decode_program(
        &[
            evaluate({
                let mut bytes = result_exp;
                bytes.extend(sysvar(41));
                bytes.push(44); // assign
                bytes
            }),
            evaluate({
                let mut bytes = result_mask;
                bytes.extend(syslist(3));
                bytes.push(44);
                bytes
            }),
            vec![4],
        ]
        .concat(),
    )
    .unwrap();

    let data = sample_data();
    let mut host = BattleScriptHost::new(&data);
    let mut vm = EventVm::new(program).with_entity(tantalus_contracts::EntityId::new(1));
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(tantalus_legacy::VariableScope::Global, 31), 640);
    assert_eq!(
        vm.variable(tantalus_legacy::VariableScope::Global, 32),
        0x30
    );
}

#[test]
fn sysvar_zero_draws_from_the_battle_random() {
    let data = BattleDataView::default();
    let kinds = [(Draw::Random8, 200i32)];
    let mut rng = SequenceRandom::with_kinds(&kinds);
    let mut host = BattleScriptHost::with_random(&data, &mut rng);
    let value = LegacyEventHost::read_system_variable(&mut host, 0).unwrap();
    assert_eq!(value, 200);
}

#[test]
fn object_members_answer_getcharacterdata() {
    use std::collections::BTreeMap;
    use tantalus_contracts::EntityId;
    use tantalus_legacy::battle::host::UnitView;

    let mut unit = UnitView {
        max_hp: 300,
        hp: 120,
        max_mp: 40,
        mp: 9,
        max_atb: 3000,
        atb: 1500,
        level: 12,
        status_invalid: 0xABCD_EF12_3456,
        status_permanent: 0x0000FF_000001,
        status_current: 0x00AA00_000055,
        element_weak: 8,
        targetable: 1,
        model: 4,
        row: 1,
        column: 2,
        category: 3,
        class: 2,
        is_player: false,
        enemy_id: 7,
        strength: 18,
        physical_defence: 10,
        steal_items: [11, 12, 13, 14],
        bonus_items: [21, 22, 255, 24],
        bonus_card: 33,
        flags: 0b101,
        out_of_reach: true,
        die_fade_rate: 32,
        motion: 9,
        current_attack: 0x56,
        pos: [10, -20, 30],
        rot: [0, 90, 0],
        bonus_exp: 640,
        bonus_gil: 250,
        trance: 100,
        trance_gauge: 0,
        ..UnitView::default()
    };
    let mut units = BTreeMap::new();
    units.insert(EntityId::new(1), unit.clone());
    let data = BattleDataView::default();
    let mut host =
        tantalus_legacy::battle::host::BattleScriptHost::with_units(&data, units.clone());
    let read = |host: &mut tantalus_legacy::battle::host::BattleScriptHost, member: u8| {
        tantalus_legacy::LegacyEventHost::read_object_member(
            host,
            tantalus_legacy::ObjectMemberRef {
                object: EntityId::new(1),
                member,
            },
        )
        .unwrap()
    };
    assert_eq!(read(&mut host, 35), 300);
    assert_eq!(read(&mut host, 36), 120);
    assert_eq!(read(&mut host, 37), 40);
    assert_eq!(read(&mut host, 38), 9);
    assert_eq!(read(&mut host, 39), 3000);
    assert_eq!(read(&mut host, 40), 1500);
    assert_eq!(read(&mut host, 41), 12);
    assert_eq!(read(&mut host, 42), 0xAB_CDEF);
    assert_eq!(read(&mut host, 43), 0x12_3456);
    assert_eq!(read(&mut host, 45), 1, "low24 of permanent");
    assert_eq!(read(&mut host, 46), 0xAA00, "high24 of current");
    assert_eq!(read(&mut host, 51), 8, "weak element");
    assert_eq!(read(&mut host, 52), 1);
    assert_eq!(read(&mut host, 57), 4);
    assert_eq!(read(&mut host, 64), 1);
    assert_eq!(read(&mut host, 65), 2);
    assert_eq!(read(&mut host, 67), 3);
    assert_eq!(read(&mut host, 68), 2, "enemy class byte");
    assert_eq!(read(&mut host, 70), 7);
    assert_eq!(read(&mut host, 72), 18);
    assert_eq!(read(&mut host, 74), 10);
    assert_eq!(read(&mut host, 100), 11);
    assert_eq!(read(&mut host, 103), 14);
    assert_eq!(read(&mut host, 105), 22);
    assert_eq!(read(&mut host, 106), 255);
    assert_eq!(read(&mut host, 108), 33);
    assert_eq!(read(&mut host, 109), 0b101);
    assert_eq!(read(&mut host, 110), 1);
    assert_eq!(read(&mut host, 111), 32);
    assert_eq!(read(&mut host, 112), 9);
    assert_eq!(read(&mut host, 113), 2, "enemy category");
    assert_eq!(read(&mut host, 114), 0x56);
    assert_eq!(read(&mut host, 140), 10);
    assert_eq!(read(&mut host, 141), -20, "negated y");
    assert_eq!(read(&mut host, 142), 30);
    assert_eq!(read(&mut host, 146), 640);
    assert_eq!(read(&mut host, 147), 250);
    assert_eq!(read(&mut host, 148), 0, "trance hidden without gauge");
    assert_eq!(read(&mut host, 149), 0);
    assert_eq!(read(&mut host, 34), 0xFF_FFFF, "unknown member default");

    // Member 113 is Humanoid for players; trance reads need the gauge.
    unit.is_player = true;
    unit.trance_gauge = 50;
    unit.trance = 77;
    units.insert(EntityId::new(1), unit);
    let mut host = tantalus_legacy::battle::host::BattleScriptHost::with_units(&data, units);
    assert_eq!(read(&mut host, 113), 1, "players read Humanoid");
    assert_eq!(read(&mut host, 148), 77);

    // Unknown objects read 0 like an unmapped script object.
    let value = tantalus_legacy::LegacyEventHost::read_object_member(
        &mut host,
        tantalus_legacy::ObjectMemberRef {
            object: EntityId::new(99),
            member: 36,
        },
    )
    .unwrap();
    assert_eq!(value, 0);
}

#[test]
fn object_member_writes_apply_setcharacterdata() {
    use std::collections::BTreeMap;
    use tantalus_contracts::EntityId;
    use tantalus_legacy::battle::host::UnitView;

    let mut units = BTreeMap::new();
    units.insert(
        EntityId::new(1),
        UnitView {
            is_player: true,
            status_invalid: 0xAAAA_BBBB_CCCC_DDDD,
            status_permanent: 0x1111_2222_3333_4444,
            status_current: 0x5555_6666_7777_8888,
            model_visible: 0b0011,
            ..UnitView::default()
        },
    );
    let data = BattleDataView::default();
    let mut host = tantalus_legacy::battle::host::BattleScriptHost::with_units(&data, units);
    let write =
        |host: &mut tantalus_legacy::battle::host::BattleScriptHost, member: u8, value: i32| {
            tantalus_legacy::LegacyEventHost::write_object_member(
                host,
                tantalus_legacy::ObjectMemberRef {
                    object: EntityId::new(1),
                    member,
                },
                value,
            )
            .unwrap()
        };
    let read = |host: &mut tantalus_legacy::battle::host::BattleScriptHost, member: u8| {
        tantalus_legacy::LegacyEventHost::read_object_member(
            host,
            tantalus_legacy::ObjectMemberRef {
                object: EntityId::new(1),
                member,
            },
        )
        .unwrap()
    };

    write(&mut host, 36, 400);
    write(&mut host, 38, 25);
    write(&mut host, 35, 999);
    write(&mut host, 40, 1234);
    assert_eq!(read(&mut host, 36), 400);
    assert_eq!(read(&mut host, 38), 25);
    assert_eq!(read(&mut host, 35), 999);
    assert_eq!(read(&mut host, 40), 1234);

    // Member 42 sign-extends into bits 24-63; member 43 truncates and
    // clears bits 32-39 (the reference masks are ported exactly).
    write(&mut host, 42, -1);
    assert_eq!(
        host.units()[&EntityId::new(1)].status_invalid,
        0xFFFF_FFFF_FFCC_DDDD,
        "member 42 sign-extends: bits 24-63 set, low 24 kept"
    );
    write(&mut host, 43, -1);
    assert_eq!(
        host.units()[&EntityId::new(1)].status_invalid,
        0xFFFF_FFFF_FFFF_FFFF,
        "member 43 fills bits 0-31 after the 42 write"
    );

    // 44-47 replace the addressed 24-bit half and keep the other half.
    write(&mut host, 44, 0xAB_CDEF);
    assert_eq!(
        host.units()[&EntityId::new(1)].status_permanent,
        0x1111_ABCD_EF33_4444,
        "high half replaced, low half kept"
    );
    write(&mut host, 45, 0x12_3456);
    assert_eq!(
        host.units()[&EntityId::new(1)].status_permanent,
        0x1111_ABCD_EF12_3456
    );
    write(&mut host, 47, 0x000001);
    assert_eq!(
        host.units()[&EntityId::new(1)].status_current & 0xFF_FFFF,
        1,
        "low half of current status"
    );

    // Elements, stats, and positions; 141 negates.
    write(&mut host, 48, 3);
    write(&mut host, 72, 21);
    write(&mut host, 140, 10);
    write(&mut host, 141, 20);
    write(&mut host, 142, -5);
    assert_eq!(read(&mut host, 48), 3);
    assert_eq!(read(&mut host, 72), 21);
    assert_eq!(read(&mut host, 140), 10);
    assert_eq!(read(&mut host, 141), -20, "y stored negated");
    assert_eq!(read(&mut host, 142), -5);

    // Mesh visibility is a bit set/clear on member 58's value.
    write(&mut host, 59, 0b1100);
    assert_eq!(read(&mut host, 58), 0b1111);
    write(&mut host, 60, 0b0001);
    assert_eq!(read(&mut host, 58), 0b1110);

    // Player writes to enemy-only members are ignored.
    write(&mut host, 100, 7);
    write(&mut host, 109, 9);
    write(&mut host, 146, 99);
    assert_eq!(read(&mut host, 100), 0xFF_FFFF, "steal read default stays");
    assert_eq!(read(&mut host, 109), 0xFF_FFFF);

    // Unknown members and unknown objects are silent no-ops.
    write(&mut host, 200, 5);
    write(&mut host, 34, 5);
    let before = host.units().clone();
    tantalus_legacy::LegacyEventHost::write_object_member(
        &mut host,
        tantalus_legacy::ObjectMemberRef {
            object: EntityId::new(77),
            member: 36,
        },
        5,
    )
    .unwrap();
    assert_eq!(host.units(), &before, "no unit changed");
}

#[test]
fn enemy_member_writes_land_and_read_back() {
    use std::collections::BTreeMap;
    use tantalus_contracts::EntityId;
    use tantalus_legacy::battle::host::UnitView;

    let mut units = BTreeMap::new();
    units.insert(
        EntityId::new(2),
        UnitView {
            is_player: false,
            ..UnitView::default()
        },
    );
    let data = BattleDataView::default();
    let mut host = tantalus_legacy::battle::host::BattleScriptHost::with_units(&data, units);
    let write =
        |host: &mut tantalus_legacy::battle::host::BattleScriptHost, member: u8, value: i32| {
            tantalus_legacy::LegacyEventHost::write_object_member(
                host,
                tantalus_legacy::ObjectMemberRef {
                    object: EntityId::new(2),
                    member,
                },
                value,
            )
            .unwrap()
        };
    write(&mut host, 100, 12);
    write(&mut host, 103, 15);
    write(&mut host, 105, 22);
    write(&mut host, 108, 33);
    write(&mut host, 109, 0b101);
    write(&mut host, 111, 16);
    write(&mut host, 113, 8);
    write(&mut host, 146, 640);
    write(&mut host, 147, 250);
    let unit = &host.units()[&EntityId::new(2)];
    assert_eq!(unit.steal_items, [12, 0, 0, 15]);
    assert_eq!(unit.bonus_items[1], 22);
    assert_eq!(unit.bonus_card, 33);
    assert_eq!(unit.flags, 0b101);
    assert_eq!(unit.die_fade_rate, 16);
    assert_eq!(unit.class, 8);
    assert_eq!(unit.bonus_exp, 640);
    assert_eq!(unit.bonus_gil, 250);
}

#[test]
fn battle_machine_snapshots_into_script_views() {
    use tantalus_contracts::EntityId;
    use tantalus_legacy::battle::host::{battle_views, phase};
    use tantalus_legacy::battle::machine::{Battle, BattleConfig, Phase};
    use tantalus_legacy::battle::{BattleUnit, StatusDataRow, StatusTable};

    fn table() -> StatusTable {
        StatusTable::new(vec![StatusDataRow::default(); 32])
    }
    let mut player = BattleUnit::new(1, true);
    player.level = 20;
    player.character_id = 5;
    player.cur_hp = 500;
    player.max_hp = 500;
    player.cur_mp = 100;
    player.max_mp = 100;
    player.strength = 30;
    player.guard_elements = tantalus_legacy::battle::Elements(3);
    let mut enemy = BattleUnit::new(16, false);
    enemy.level = 15;
    enemy.is_undead = true;
    enemy.cur_hp = 300;
    enemy.max_hp = 300;
    enemy.physical_defence = 10;
    let mut battle = Battle::new(vec![player, enemy], BattleConfig::default(), table());
    battle.bonus_exp = 640;
    battle.running.push(0);

    let (data, units) = battle_views(&battle);
    assert_eq!(data.btl_phase, phase::NORMAL);
    assert_eq!(data.bonus_exp, 640);
    assert!(data.seq_busy, "a running command means busy");
    assert_eq!(data.battle_result, 0, "no result yet");

    let player_view = &units[&EntityId::new(1)];
    assert_eq!(player_view.hp, 500);
    assert_eq!(player_view.level, 20);
    assert_eq!(player_view.category, 5, "player category = character id");
    assert_eq!(player_view.element_invalid, 3);
    assert!(player_view.is_player);
    let enemy_view = &units[&EntityId::new(16)];
    assert_eq!(enemy_view.physical_defence, 10);
    assert_eq!(enemy_view.class, 16, "undead maps to the Undead category");

    // A decoded script reads the snapshot through the host: unit 16's hp
    // into a global.
    let result = global_int24(31);
    let program = decode_program(
        &[
            evaluate({
                let mut bytes = result;
                bytes.extend(vec![120, 16, 36]); // getvobj(entity 16, member 36)
                bytes.push(44);
                bytes
            }),
            vec![4],
        ]
        .concat(),
    )
    .unwrap();
    let mut host = tantalus_legacy::battle::host::BattleScriptHost::with_units(&data, units);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(1));
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(tantalus_legacy::VariableScope::Global, 31), 300);

    // Victory phase maps to the reference phase number.
    battle.phase = Phase::Victory;
    battle.result = Some(1);
    let (data, _) = battle_views(&battle);
    assert_eq!(data.btl_phase, phase::VICTORY);
    assert_eq!(data.battle_result, 1);
    assert!(data.result_mode);
}

#[test]
fn script_writes_apply_back_onto_the_machine() {
    use tantalus_contracts::EntityId;
    use tantalus_legacy::battle::host::{apply_unit_views, battle_views};
    use tantalus_legacy::battle::machine::{Battle, BattleConfig};
    use tantalus_legacy::battle::{BattleUnit, StatusDataRow, StatusTable};

    let mut player = BattleUnit::new(1, true);
    player.cur_hp = 500;
    player.max_hp = 500;
    player.guard_elements = tantalus_legacy::battle::Elements(3);
    let mut enemy = BattleUnit::new(16, false);
    enemy.cur_hp = 300;
    enemy.max_hp = 300;
    let mut battle = Battle::new(
        vec![player, enemy],
        BattleConfig::default(),
        StatusTable::new(vec![StatusDataRow::default(); 32]),
    );

    // Snapshot, mutate through the host the way a battle script would, apply.
    let (_, mut units) = battle_views(&battle);
    {
        let unit = units.get_mut(&EntityId::new(16)).unwrap();
        unit.hp = 120;
        unit.targetable = 0;
        unit.element_weak = 8;
        unit.status_current = 0xFF;
    }
    // Views for an unknown entity are ignored.
    units.insert(EntityId::new(99), Default::default());
    apply_unit_views(&mut battle, &units);

    assert_eq!(battle.units[1].cur_hp, 120, "script hp write lands");
    assert!(!battle.sched[1].targetable, "targetable write lands");
    assert_eq!(battle.units[1].weak_elements.0, 8);
    assert_eq!(battle.units[1].status_current, 0xFF);
    assert_eq!(battle.units[0].cur_hp, 500, "untouched unit unchanged");
    assert_eq!(
        battle.units[0].guard_elements.0, 3,
        "untouched element kept"
    );

    // A second snapshot observes the applied writes: the script loop is
    // round-trip consistent.
    let (_, views) = battle_views(&battle);
    assert_eq!(views[&EntityId::new(16)].hp, 120);
    assert_eq!(views[&EntityId::new(16)].targetable, 0);
}

#[test]
fn invocation_command_parameters_override_sysvars_28_29() {
    let mut data = sample_data();
    let mut host = BattleScriptHost::new(&data);
    let read = |host: &mut BattleScriptHost, code: u8| {
        tantalus_legacy::LegacyEventHost::read_system_variable(host, code).unwrap()
    };
    // No invocation context: the resolving command answers.
    assert_eq!(read(&mut host, 28), 3);
    assert_eq!(read(&mut host, 29), 0x1D);

    // Invocation parameters (gExec.level <= 2 path) win.
    let mut host = BattleScriptHost::new(&data).with_cmd_params(9, 0x55);
    assert_eq!(read(&mut host, 28), 9, "_btlCmdPrmCmd");
    assert_eq!(read(&mut host, 29), 0x55, "_btlCmdPrmSub");

    // Invocation parameters also apply with no resolving command.
    data.current_command = None;
    let mut host = BattleScriptHost::new(&data).with_cmd_params(4, 7);
    assert_eq!(read(&mut host, 28), 4);
    assert_eq!(read(&mut host, 29), 7);
}
