use super::super::{BattleTables, KnownRowCheck};
use super::*;

fn int(value: i64) -> Tree {
    Tree::Int(value)
}

fn bytes(element: &str, values: &[i64]) -> Tree {
    Tree::Array {
        element: element.to_owned(),
        items: values.iter().map(|v| int(*v)).collect(),
    }
}

fn matrix(element: &str, rows: &[&[i64]]) -> Tree {
    Tree::Array {
        element: format!("{element}[,]"),
        items: rows.iter().map(|r| bytes(element, r)).collect(),
    }
}

fn menu_statics(normal: &[&[i64]], trance: &[&[i64]]) -> BTreeMap<String, Tree> {
    BTreeMap::from([
        (
            "_FF9BMenu_MenuNormal".to_owned(),
            matrix("FF9.command_tags", normal),
        ),
        (
            "_FF9BMenu_MenuTrance".to_owned(),
            matrix("System.Byte", trance),
        ),
    ])
}

#[test]
fn command_menus_pair_the_two_tables_by_menu_type() {
    let statics = menu_statics(&[&[2, 25], &[22, 13]], &[&[2, 26, 2], &[23, 13, 1]]);
    let rows = command_menus(&statics).unwrap();
    assert_eq!(
        rows,
        vec![
            CommandMenuRow {
                menu_type: 0,
                normal: [2, 25],
                trance: [2, 26],
                trance_third: 2,
            },
            CommandMenuRow {
                menu_type: 1,
                normal: [22, 13],
                trance: [23, 13],
                trance_third: 1,
            },
        ]
    );
}

#[test]
fn command_menus_refuse_wrong_shapes_and_unsupported_values() {
    let narrow = menu_statics(&[&[2]], &[&[2, 26, 2]]);
    assert!(command_menus(&narrow).is_err());
    let uneven = menu_statics(&[&[2, 25], &[22, 13]], &[&[2, 26, 2]]);
    assert!(
        command_menus(&uneven)
            .unwrap_err()
            .detail
            .contains("differ in length")
    );
    let too_big = menu_statics(&[&[2, 300]], &[&[2, 26, 2]]);
    assert!(command_menus(&too_big).is_err());
    let mut unsupported = menu_statics(&[&[2, 25]], &[&[2, 26, 2]]);
    unsupported.insert(
        "_FF9BMenu_MenuNormal".to_owned(),
        Tree::Array {
            element: "FF9.command_tags[,]".to_owned(),
            items: vec![Tree::Unsupported("call".to_owned())],
        },
    );
    assert!(command_menus(&unsupported).is_err());
    assert!(command_menus(&BTreeMap::new()).is_err());
}

fn run_with_locals(locals: Vec<(u16, Tree)>) -> MethodRun {
    MethodRun {
        locals: locals.into_iter().collect(),
        statics: BTreeMap::new(),
        calls: Vec::new(),
        stores: Vec::new(),
        stop: None,
        returned: None,
        steps: 0,
    }
}

#[test]
fn array_local_demands_exactly_one_distinct_array_of_the_element() {
    let table = bytes("System.Byte", &[9, 5, 6]);
    let run = run_with_locals(vec![(0, table.clone()), (1, int(4))]);
    assert_eq!(array_local(&run, "t", "System.Byte").unwrap().len(), 3);

    let aliased = run_with_locals(vec![(0, table.clone()), (1, table.clone())]);
    assert!(array_local(&aliased, "t", "System.Byte").is_ok());

    let ambiguous = run_with_locals(vec![(0, table), (1, bytes("System.Byte", &[1]))]);
    assert!(array_local(&ambiguous, "t", "System.Byte").is_err());
    assert!(array_local(&run, "t", "PLAYER_INFO").is_err());
}

#[test]
fn jagged_rows_must_have_the_stated_width_and_fit_bytes() {
    let rows = vec![bytes("System.Byte", &[1, 112, 88, 149, 255])];
    assert_eq!(
        jagged("equip", &rows, 5).unwrap(),
        vec![vec![1, 112, 88, 149, 255]]
    );
    assert!(jagged("equip", &rows, 4).is_err());
    assert!(jagged("equip", &[bytes("System.Byte", &[256])], 1).is_err());
    assert!(jagged("equip", &[int(1)], 1).is_err());
}

fn sample_tables() -> BattleTables {
    let mut tables = BattleTables {
        actions: Vec::new(),
        status_data: Vec::new(),
        status_sets: Vec::new(),
        items: Vec::new(),
        item_use: Vec::new(),
        weapons: Vec::new(),
        armor: Vec::new(),
        equip_bonus: Vec::new(),
        learnable_abilities: Vec::new(),
        support_abilities: Vec::new(),
        shops: Vec::new(),
        synthesis: Vec::new(),
        commands: Vec::new(),
        command_abilities: Vec::new(),
        level_base: Vec::new(),
        level_hp_mp: Vec::new(),
        level_exp: Vec::new(),
        player: PlayerTables::default(),
    };
    tables.player.default_equipment = vec![EquipmentSetRow {
        id: 0,
        weapon: 1,
        head: 112,
        wrist: 88,
        armor: 149,
        accessory: 255,
    }];
    tables
}

fn player_checks(tables: &BattleTables) -> Vec<KnownRowCheck> {
    let mut checks = Vec::new();
    super::super::player_known::run(tables, &mut checks);
    checks
}

#[test]
fn known_row_checks_pass_on_matching_rows_and_fail_loudly_on_missing_or_wrong_ones() {
    let tables = sample_tables();
    let checks = player_checks(&tables);
    let equipment: Vec<_> = checks
        .iter()
        .filter(|c| c.table == "default_equipment")
        .collect();
    assert_eq!(equipment.len(), 16);
    assert!(equipment[0].passed, "{:?}", equipment[0]);
    // Rows the reader did not produce must fail, not be skipped.
    assert!(equipment[1..].iter().all(|c| !c.passed));
    assert!(
        checks
            .iter()
            .filter(|c| c.table == "command_menus")
            .all(|c| !c.passed)
    );

    let mut wrong = sample_tables();
    wrong.player.default_equipment[0].head = 113;
    let failed = player_checks(&wrong)
        .into_iter()
        .find(|c| c.table == "default_equipment" && c.row == 0)
        .unwrap();
    assert!(!failed.passed);
    assert_eq!(failed.expected, "[1, 112, 88, 149, 255]");
    assert_eq!(failed.actual, "[1, 113, 88, 149, 255]");
}

#[test]
fn player_tables_round_trip_through_json_and_default_when_absent() {
    let tables = sample_tables();
    let text = serde_json::to_string(&tables).unwrap();
    let back: BattleTables = serde_json::from_str(&text).unwrap();
    assert_eq!(back.player, tables.player);
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value.as_object_mut().unwrap().remove("player");
    let older: BattleTables = serde_json::from_value(value).unwrap();
    assert_eq!(older.player, PlayerTables::default());
}
