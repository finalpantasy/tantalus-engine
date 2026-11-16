use super::*;

fn int(value: i64) -> Tree {
    Tree::Int(value)
}

fn object(type_name: &str, args: &[(&str, Tree)]) -> Tree {
    Tree::Object {
        type_name: type_name.to_owned(),
        args: args
            .iter()
            .map(|(name, tree)| ((*name).to_owned(), tree.clone()))
            .collect(),
    }
}

fn array(element: &str, items: Vec<Tree>) -> Tree {
    Tree::Array {
        element: element.to_owned(),
        items,
    }
}

fn bytes(values: &[i64]) -> Tree {
    array("System.Byte", values.iter().map(|v| int(*v)).collect())
}

fn cmd_info(vfx: i64) -> Tree {
    object(
        "CMD_INFO",
        &[
            ("cursor", int(3)),
            ("def_cur", int(1)),
            ("sub_win", int(1)),
            ("vfx_no", int(vfx)),
            ("sfx_no", int(0)),
            ("dead", int(1)),
            ("def_cam", int(0)),
            ("def_dead", int(0)),
        ],
    )
}

fn btl_ref(power: i64) -> Tree {
    object(
        "BTL_REF",
        &[
            ("prog_no", int(10)),
            ("power", int(power)),
            ("attr", int(0)),
            ("rate", int(255)),
        ],
    )
}

#[test]
fn actions_keep_raw_named_fields_and_index_ids() {
    let row = object(
        "AA_DATA",
        &[
            ("Info", cmd_info(9)),
            ("Ref", btl_ref(16)),
            ("Category", int(71)),
            ("AddNo", int(0)),
            ("MP", int(6)),
            ("Type", int(1)),
            ("Vfx2", int(8)),
            ("Name", int(5)),
        ],
    );
    let parsed = parse_actions(&array("AA_DATA", vec![row.clone(), row])).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[1].id, 1);
    assert_eq!(parsed[1].info.vfx_no, 9);
    assert_eq!(parsed[1].btl_ref.power, 16);
    assert_eq!(parsed[1].category, 71);
    assert_eq!(parsed[1].mp, 6);
    assert_eq!(parsed[1].name_arg, 5);
}

#[test]
fn rejects_wrong_constructor_type_and_oversized_values() {
    let wrong = array("AA_DATA", vec![object("OTHER", &[])]);
    assert!(
        parse_actions(&wrong)
            .unwrap_err()
            .detail
            .contains("expected")
    );
    let big = object(
        "DEF_PARAMS",
        &[
            ("p_def", int(256)),
            ("p_ev", int(0)),
            ("m_def", int(0)),
            ("m_ev", int(0)),
        ],
    );
    assert!(parse_armor(&array("DEF_PARAMS", vec![big])).is_err());
}

#[test]
fn status_data_and_status_sets_read_unsigned_words() {
    let row = object(
        "STAT_DATA",
        &[
            ("priority", int(1)),
            ("opr_cnt", int(0)),
            ("conti_cnt", int(0)),
            ("clear", int(4_026_506_810)),
            ("invalid", int(4_294_966_975)),
        ],
    );
    let parsed = parse_status_data(&array("STAT_DATA", vec![row])).unwrap();
    assert_eq!(parsed[0].clear, 4_026_506_810);
    assert_eq!(parsed[0].invalid, 4_294_966_975);
    let sets = parse_u32_array(
        "add_status",
        &array(
            "System.UInt32",
            vec![int(0), int(262_144), int(2_147_483_649)],
        ),
    )
    .unwrap();
    assert_eq!(sets, vec![0, 262_144, 2_147_483_649]);
    assert!(parse_u32_array("x", &array("System.Int32", vec![int(-1)])).is_err());
}

#[test]
fn items_decode_ability_triples() {
    let row = object(
        "FF9ITEM_DATA",
        &[
            ("name", int(7)),
            ("help", int(16)),
            ("price", int(320)),
            ("equip", int(2048)),
            ("shape", int(1)),
            ("color", int(0)),
            ("eq_lv", int(1)),
            ("bonus", int(0)),
            ("ability", bytes(&[101, 0, 0])),
            ("type", int(128)),
            ("sort", int(22)),
            ("pad", int(0)),
        ],
    );
    let parsed = parse_items(&array("FF9ITEM_DATA", vec![row.clone()])).unwrap();
    assert_eq!(parsed[0].ability, [101, 0, 0]);
    assert_eq!(parsed[0].kind, 128);
    assert_eq!(parsed[0].price, 320);
    let mut short = row.clone();
    if let Tree::Object { args, .. } = &mut short {
        args.iter_mut().find(|(n, _)| n == "ability").unwrap().1 = bytes(&[1, 2]);
    }
    assert!(parse_items(&array("FF9ITEM_DATA", vec![short])).is_err());
}

#[test]
fn weapons_resolve_model_number_like_the_constructor() {
    let weapon = |name: Tree| {
        object(
            "WEAPON",
            &[
                ("category", int(5)),
                ("add_no", int(9)),
                ("model_name", name),
                ("Ref", btl_ref(14)),
                ("offset", array("System.Int16", vec![int(-32), int(200)])),
            ],
        )
    };
    let geo = Tree::Map(vec![
        (int(10), Tree::Str("GEO_A".to_owned())),
        (int(20), Tree::Str("GEO_B".to_owned())),
        (int(30), Tree::Str("GEO_B".to_owned())),
    ]);
    let parsed = parse_weapons(
        &array(
            "WEAPON",
            vec![
                weapon(Tree::Str("GEO_B".to_owned())),
                weapon(Tree::Str("MISSING".to_owned())),
                weapon(Tree::Null),
            ],
        ),
        &geo,
    )
    .unwrap();
    assert_eq!((parsed[0].model_no, parsed[0].model_found), (20, true));
    assert_eq!((parsed[1].model_no, parsed[1].model_found), (0, false));
    assert_eq!(
        (parsed[2].model_name.clone(), parsed[2].model_found),
        (None, false)
    );
    assert_eq!(parsed[0].offset, [-32, 200]);
}

#[test]
fn equipment_bonus_flattens_defense_attributes() {
    let row = object(
        "EQUIP_PRIVILEGE",
        &[
            ("dex", int(1)),
            ("str", int(2)),
            ("mgc", int(3)),
            ("wpr", int(4)),
            (
                "def_attr",
                object(
                    "DEF_ATTR",
                    &[
                        ("invalid", int(8)),
                        ("absorb", int(0)),
                        ("half", int(16)),
                        ("weak", int(32)),
                    ],
                ),
            ),
            ("p_up_attr", int(5)),
        ],
    );
    let parsed = parse_equip_bonus(&array("EQUIP_PRIVILEGE", vec![row])).unwrap();
    assert_eq!(
        (
            parsed[0].str_,
            parsed[0].half,
            parsed[0].weak,
            parsed[0].p_up_attr
        ),
        (2, 16, 32, 5)
    );
}

#[test]
fn abilities_shops_and_synthesis() {
    let pa = |id, ap| object("PA_DATA", &[("id", int(id)), ("max_ap", int(ap))]);
    let sets = array(
        "PA_DATA[]",
        vec![
            array("PA_DATA", vec![pa(101, 40), pa(102, 40)]),
            array("PA_DATA", vec![pa(7, 9)]),
        ],
    );
    let parsed = parse_learnable(&sets).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(
        parsed[0][1],
        AbilityPaRow {
            id: 102,
            max_ap: 40
        }
    );

    let sa = object(
        "SA_DATA",
        &[
            ("category", int(0)),
            ("capa_val", int(15)),
            ("name", int(0)),
            ("help", int(0)),
            ("help_size", int(8301)),
        ],
    );
    let parsed = parse_support_abilities(&array("SA_DATA", vec![sa])).unwrap();
    assert_eq!((parsed[0].capa_val, parsed[0].help_size), (15, 8301));

    let shops = array("System.Byte[]", vec![bytes(&[1, 2, 255]), bytes(&[])]);
    assert_eq!(parse_shops(&shops).unwrap(), vec![vec![1, 2, 255], vec![]]);

    let mix = object(
        "FF9MIX_DATA",
        &[
            ("price", int(300)),
            ("src", bytes(&[1, 2])),
            ("dst", int(7)),
            ("shop", int(79)),
        ],
    );
    let parsed = parse_synthesis(&array("FF9MIX_DATA", vec![mix])).unwrap();
    assert_eq!((parsed[0].src, parsed[0].dst), ([1, 2], 7));
}

#[test]
fn commands_accept_the_nested_type_name() {
    let command = object(
        "FF9COMMAND",
        &[
            ("name", int(1)),
            ("help", int(1)),
            ("help_size", int(8293)),
            ("type", int(0)),
            ("count", int(0)),
            ("ability", int(176)),
        ],
    );
    let parsed = parse_commands(&array("FF9COMMAND", vec![command])).unwrap();
    assert_eq!(parsed[0].ability, 176);
    assert_eq!(parsed[0].help_size, 8293);
}

#[test]
fn level_tables() {
    let base = object(
        "FF9LEVEL_BASE",
        &[
            ("dex", int(23)),
            ("str", int(21)),
            ("mgc", int(18)),
            ("wpr", int(23)),
            ("cap", int(18)),
        ],
    );
    let parsed = parse_level_base(&array("FF9LEVEL_BASE", vec![base])).unwrap();
    assert_eq!((parsed[0].dex, parsed[0].cap), (23, 18));
    let hp = object("FF9LEVEL_HPMP", &[("hp", int(250)), ("mp", int(200))]);
    let parsed = parse_level_hp_mp(&array("FF9LEVEL_HPMP", vec![hp])).unwrap();
    assert_eq!((parsed[0].hp, parsed[0].mp), (250, 200));
    assert_eq!(
        parse_level_exp(&array("System.UInt64", vec![int(0), int(16), int(47)])).unwrap(),
        vec![0, 16, 47]
    );
}

#[test]
fn unsupported_static_fields_are_refused() {
    let mut statics = BTreeMap::new();
    statics.insert(
        "aa_data".to_owned(),
        array("AA_DATA", vec![Tree::Unsupported("call".to_owned())]),
    );
    let error = field(&statics, "FF9BattleDB", "aa_data").unwrap_err();
    assert!(error.detail.contains("cannot read"));
    assert!(field(&statics, "FF9BattleDB", "missing").is_err());
}

/// Reads the installed assembly named by `TANTALUS_ASSEMBLY_CSHARP`; run with
/// `cargo test -p tantalus-assets --lib real_install -- --ignored`.
#[test]
#[ignore = "needs an owned install: set TANTALUS_ASSEMBLY_CSHARP"]
fn real_install_matches_known_rows() {
    let path = std::env::var("TANTALUS_ASSEMBLY_CSHARP").expect("set TANTALUS_ASSEMBLY_CSHARP");
    let bytes = std::fs::read(path).unwrap();
    let assembly = Assembly::parse(&bytes).unwrap();
    let tables = extract_battle_tables(&assembly).unwrap();
    assert_eq!(tables.actions.len(), 192);
    assert_eq!(tables.items.len(), 256);
    let checks = verify_known_rows(&tables);
    assert!(checks.len() >= 180);
    let player = &tables.player;
    assert_eq!(player.command_menus.len(), 20);
    assert_eq!(player.player_slots.len(), 9);
    assert_eq!(player.default_equipment.len(), 16);
    assert_eq!(player.default_names.len(), 7);
    assert_eq!(player.text_bindings.len(), 12);
    assert_eq!(player.new_game.party.len(), 4);
    assert!(
        checks.iter().all(|c| c.passed),
        "{:?}",
        checks.iter().filter(|c| !c.passed).collect::<Vec<_>>()
    );
}
