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
