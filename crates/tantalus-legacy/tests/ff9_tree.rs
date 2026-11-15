//! Synthetic-data tests for the legacy FF9 save tree. No real save is read anywhere.

use tantalus_legacy::saves::ff9_tree::*;
use tantalus_legacy::{BASE_SAVE_BLOCK_BYTES, SimpleJsonNode};

fn populated() -> Ff9SaveTree {
    let mut t = Ff9SaveTree::blank();
    t.state = StateSection {
        mode: 5,
        prev_mode: 3,
        fld_map_no: 1234,
        fld_loc_no: -2,
        btl_map_no: 77,
        btl_sub_map_no: -3,
        wld_map_no: 9,
        wld_loc_no: 10,
        time_counter: 3600.5,
        timer_control: true,
        timer_display: false,
    };
    t.event.step_count = 424_242;
    let mut global = EventGlobal::zeroed();
    global.set_scenario_counter(9860);
    global.set_field_entrance(0x1234);
    global.set_chocograph_opened(0x00AB_CDEF);
    global.set_chocograph_found(0x0012_3456);
    global.set_chocograph_ability(5);
    global.as_bytes_mut()[900] = 0xFF;
    t.event.event_global = global;
    t.mini_game.s_win = 12;
    t.mini_game.s_lose = -1;
    t.mini_game.s_draw = 3;
    t.mini_game.cards[0] = CardSlot {
        id: 7,
        side: 0,
        atk: 4,
        card_type: 2,
        pdef: 1,
        mdef: 2,
        cpoint: 3,
        arrow: 0xA5,
    };
    for (i, player) in t.common.players.iter_mut().enumerate() {
        let n = i as u8 + 1;
        player.name = format!("Synthetic {n} \u{2713}");
        player.level = n * 3;
        player.exp = 1000 * u32::from(n);
        player.cur = Resources {
            hp: 100 * u16::from(n),
            mp: -(i16::from(n)),
            at: 5,
            at_coef: -4,
            capa: n,
        };
        player.max = Resources {
            hp: 9999,
            mp: 999,
            at: 1,
            at_coef: 2,
            capa: 99,
        };
        player.elem = [n, n + 1, n + 2, n + 3];
        player.defence = [9, 8, 7, 6];
        player.basis_max_hp = 4000;
        player.basis_max_mp = 400;
        player.basis_stats = [20, 21, 22, 23];
        player.info = [n, n, 1, 2, 3, 4];
        player.status = 0x80 | n;
        player.equip = [1, 2, 3, 4, 5];
        player.bonus = [100, 200, 300, 65535];
        player.pa[47] = n;
        player.sa = [0xDEAD_BEEF, u32::from(n)];
    }
    t.common.slot = [0, 1, 255, 3];
    t.common.gil = 4_000_000_000;
    t.common.escape_no = 321;
    t.common.summon_flag = 0xFFFF;
    t.common.frog_no = 99;
    t.common.steal_no = 98;
    t.common.dragon_no = 97;
    t.common.items[3] = (12, 99);
    t.common.rare_items[63] = 0b0101_0011;
    t.setting.sound = 1;
    t.setting.move_ = u64::MAX;
    t.setting.control_data_keyboard[9] = 0x7FFF_FFFF;
    t.setting.control_data_joystick[0] = "Joystick Button 3".to_owned();
    t.auto_save_bgm_id = 55;
    t.world = WorldSection {
        rotation_max: 1.5,
        upper_counter: 8,
        upper_counter_speed: -12,
        upper_counter_force: true,
        rotation: 2.25,
        rotation_rev: -3.5,
        hintmap: 24,
    };
    t.achievement.blk_mag_no = 3;
    t.achievement.ate_check[99] = 17;
    t.achievement.abilities[0] = 5;
    t.achievement.quadmist_win_list[299] = 42;
    t.achievement.synthesis_count = 6;
    t.achievement.auction_time = 7;
    t.achievement.stiltzkin_buy = 8;
    t.reserved_91000[127] = 91;
    t.reserved_92000[0] = 92;
    t.reserved_93000[1] = 93;
    t.common_94000.player_bonus = [1, 2, 3, 4, 5, 6, 7, 8, 9];
    t.common_94000.reserved[118] = 94;
    t.setting_95000.time = 12_345.678;
    t.setting_95000.reserved[125] = 95;
    t.sound_96000[5] = 96;
    t.world_97000[6] = 97;
    t.achievement_98000.abnormal_status = 0xF0F0_F0F0;
    t.achievement_98000.summons = [
        true, false, true, false, true, true, false, false, true, false, true, false, true, true,
        false, true,
    ];
    t.achievement_98000.reserved[122] = 98;
    t.other_99000[383] = 99;
    t
}

#[test]
fn payload_fits_one_block_and_matches_schema_size() {
    let bytes = Ff9SaveTree::blank()
        .encode_payload(KeyOrder::Ordinal)
        .unwrap();
    assert_eq!(bytes.len(), Ff9SaveTree::PAYLOAD_BYTES);
    assert!(bytes.len() + 4 <= BASE_SAVE_BLOCK_BYTES);
}

#[test]
fn decode_of_encode_is_identity_for_both_key_orders() {
    for order in [KeyOrder::Ordinal, KeyOrder::CultureApprox] {
        let tree = populated();
        let bytes = tree.encode_payload(order).unwrap();
        let back = Ff9SaveTree::decode_payload(&bytes, order).unwrap();
        assert_eq!(back, tree, "{order:?}");
        assert_eq!(back.encode_payload(order).unwrap(), bytes, "{order:?}");
        let blank = Ff9SaveTree::blank();
        let blank_bytes = blank.encode_payload(order).unwrap();
        assert_eq!(
            Ff9SaveTree::decode_payload(&blank_bytes, order).unwrap(),
            blank
        );
    }
}

#[test]
fn key_order_changes_the_layout_of_mixed_case_sections() {
    let tree = populated();
    let ordinal = tree.encode_payload(KeyOrder::Ordinal).unwrap();
    let culture = tree.encode_payload(KeyOrder::CultureApprox).unwrap();
    assert_ne!(ordinal, culture);
    assert_eq!(ordinal.len(), culture.len());
    // The first sections (state, event, minigame, common) have no case-sensitive ties.
    assert_eq!(ordinal[..4096], culture[..4096]);
}

/// Hand-built blob: the first two sections are written byte by byte from the documented layout
/// (sorted keys, little endian), the remainder is the blank tree.
#[test]
fn hand_built_blob_decodes_and_re_encodes_byte_for_byte() {
    let blank_bytes = Ff9SaveTree::blank()
        .encode_payload(KeyOrder::Ordinal)
        .unwrap();
    let mut blob = Vec::new();
    // 10000_State keys in ordinal order: btlMapNo, btlSubMapNo, fldLocNo, fldMapNo, mode,
    // prevMode, timeCounter, timerControl, timerDisplay, wldLocNo, wldMapNo.
    blob.extend(300_i16.to_le_bytes());
    blob.push((-2_i8) as u8);
    blob.extend(5_i16.to_le_bytes());
    blob.extend(0x0102_i16.to_le_bytes());
    blob.push(7);
    blob.push(8);
    blob.extend(1.0_f32.to_le_bytes());
    blob.push(1);
    blob.push(0);
    blob.extend(11_i16.to_le_bytes());
    blob.extend(12_i16.to_le_bytes());
    assert_eq!(blob.len(), 19);
    // 20000_Event: gEventGlobal (4096-byte NUL padded base64) then gStepCount.
    let mut event_global = vec![0u8; 2048];
    event_global[0] = 0x84; // ScenarioCounter low byte
    event_global[1] = 0x26; // ScenarioCounter high byte: 0x2684 = 9860
    event_global[2] = 0x34;
    event_global[3] = 0x12;
    let base64 = EventGlobal::from_bytes(event_global).unwrap().to_base64();
    assert_eq!(base64.len(), 2732);
    assert!(base64.starts_with("hCY0Eg"));
    let mut field = base64.into_bytes();
    field.resize(4096, 0);
    blob.extend(field);
    blob.extend(31_337_i32.to_le_bytes());
    blob.extend(&blank_bytes[blob.len()..]);

    let tree = Ff9SaveTree::decode_payload(&blob, KeyOrder::Ordinal).unwrap();
    assert_eq!(tree.state.btl_map_no, 300);
    assert_eq!(tree.state.btl_sub_map_no, -2);
    assert_eq!(tree.state.fld_loc_no, 5);
    assert_eq!(tree.state.fld_map_no, 0x0102);
    assert_eq!((tree.state.mode, tree.state.prev_mode), (7, 8));
    assert_eq!(tree.state.time_counter, 1.0);
    assert!(tree.state.timer_control && !tree.state.timer_display);
    assert_eq!((tree.state.wld_loc_no, tree.state.wld_map_no), (11, 12));
    assert_eq!(tree.event.step_count, 31_337);
    assert_eq!(tree.event.event_global.scenario_counter(), 9860);
    assert_eq!(tree.event.event_global.field_entrance(), 0x1234);
    assert_eq!(tree.encode_payload(KeyOrder::Ordinal).unwrap(), blob);
}

#[test]
fn event_global_named_ranges_round_trip_through_base64() {
    let mut g = EventGlobal::zeroed();
    g.set_scenario_counter(0xBEEF);
    g.set_field_entrance(0xCAFE);
    assert_eq!(&g.as_bytes()[..4], &[0xEF, 0xBE, 0xFE, 0xCA]);
    g.set_chocograph_opened(0x0003_0201);
    g.set_chocograph_found(0x0006_0504);
    g.set_chocograph_ability(5);
    assert_eq!(&g.as_bytes()[184..=191], &[1, 2, 3, 4, 5, 6, 0, 5]);
    assert_eq!(g.chocograph_opened(), 0x0003_0201);
    assert_eq!(g.chocograph_found(), 0x0006_0504);
    let back = EventGlobal::from_base64(&g.to_base64()).unwrap();
    assert_eq!(back, g);
    assert_eq!(back.scenario_counter(), 0xBEEF);

    // Treasure Hunter: 1 point per bit in 896..=960 and 966..=975, 2 per bit in 182..=186.
    let mut g = EventGlobal::zeroed();
    g.as_bytes_mut()[896] = 0b0000_0111;
    g.as_bytes_mut()[975] = 0xFF;
    g.as_bytes_mut()[961] = 0xFF; // outside every range
    g.as_bytes_mut()[182] = 0b11;
    g.as_bytes_mut()[186] = 0b1;
    assert_eq!(g.treasure_hunter_points(), 3 + 8 + 2 * 2 + 2);
    assert_eq!(g.legacy_ability_usage(0), None);
    assert_eq!(g.legacy_ability_usage(192), None);
    g.as_bytes_mut()[1100 + 191] = 9;
    assert_eq!(g.legacy_ability_usage(191), Some(9));
}

#[test]
fn event_global_rejects_wrong_sizes_and_bad_base64() {
    assert!(EventGlobal::from_bytes(vec![0; 2047]).is_err());
    assert!(EventGlobal::from_base64("AAAA").is_err());
    assert!(EventGlobal::from_base64("not base64!").is_err());
}

#[test]
fn malformed_trees_and_payloads_are_rejected() {
    let bytes = Ff9SaveTree::blank()
        .encode_payload(KeyOrder::Ordinal)
        .unwrap();
    assert!(Ff9SaveTree::decode_payload(&bytes[1..], KeyOrder::Ordinal).is_err());
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(Ff9SaveTree::decode_payload(&longer, KeyOrder::Ordinal).is_err());
    let mut tree = Ff9SaveTree::blank();
    tree.common.items.pop();
    assert!(tree.encode_payload(KeyOrder::Ordinal).is_err());
    let mut tree = Ff9SaveTree::blank();
    tree.common.players[0].name = "x".repeat(129);
    assert!(tree.encode_payload(KeyOrder::Ordinal).is_err());
    // Corrupt the base64 text of gEventGlobal (first byte of the string field at offset 19).
    let mut corrupt = bytes;
    corrupt[19] = b'!';
    assert!(Ff9SaveTree::decode_payload(&corrupt, KeyOrder::Ordinal).is_err());
}

fn sample_player(extended: bool) -> MemoriaPlayer {
    MemoriaPlayer {
        name: "Synthetic".to_owned(),
        category: 1,
        level: 12,
        exp: 3456,
        cur: [100, 20, 3, -1, 40],
        max: [200, 30, 4, -2, 50],
        trance: 6,
        web_bone: 7,
        elem: [1, 2, 3, 4],
        defence: [5, 6, 7, 8],
        basis: [900, 90, 10, 11, 12, 13],
        info: [3, 4, 1, 0, 1, 2],
        sub_replaced: extended,
        status: 5_000_000_000,
        equip: [1, 255, 3, 4, 5],
        bonus: [1, 2, 3, 4, 70],
        abilities: if extended {
            MemoriaAbilities::Extended {
                pa: vec![(10, 5), (11, 255)],
                sa: vec![3, 9],
            }
        } else {
            MemoriaAbilities::Plain {
                pa: vec![0, 1, 2],
                sa: vec![5, 6],
            }
        },
        max_hp_limit: 9999,
        max_mp_limit: 999,
        max_damage_limit: 9999,
        max_mp_damage_limit: 999,
        basis_capa: 12,
        basis_max_capa: 99,
    }
}

fn sample_sidecar() -> MemoriaExtra {
    let mut global = EventGlobal::zeroed();
    global.set_scenario_counter(10_000);
    MemoriaExtra {
        setting_time: MemoriaExtra::time_leaf(98_765.432_1),
        event: MemoriaEvent {
            step_count: 17,
            event_global: global,
            ability_usage: vec![(3, 4), (200, 1)],
            script_vector: vec![(5, vec![1, 2, 3])],
            script_dictionary: vec![(6, vec![(1, 100), (2, -200)])],
        },
        common: MemoriaCommon {
            players: vec![sample_player(false), sample_player(true)],
            slot: vec![0, 255, 255, 255],
            escape_no: 3,
            summon_flag: 4,
            gil: 3_000_000_000,
            frog_no: 5,
            steal_no: 6,
            kills_per_category: vec![(0, 1), (3, 40)],
            kills_per_model: vec![(250, 2)],
            items: vec![(1, 99), (300, 4)],
            rare_items_ex: vec![(5, true, false), (6, false, true)],
            battle_no: 7,
        },
        mini_game: MemoriaMiniGame {
            s_win: 1,
            s_lose: 2,
            s_draw: 3,
            cards: vec![CardSlot {
                id: 100,
                side: 0,
                atk: 5,
                card_type: 1,
                pdef: 2,
                mdef: 3,
                cpoint: 4,
                arrow: 255,
            }],
        },
    }
}

#[test]
fn sidecar_round_trips_and_re_encodes_byte_for_byte() {
    let sidecar = sample_sidecar();
    let blob = sidecar.encode().unwrap();
    let back = MemoriaExtra::decode(&blob).unwrap();
    assert_eq!(back, sidecar);
    assert_eq!(back.encode().unwrap(), blob);
    assert_eq!(back.setting_time_seconds().unwrap(), 98_765.432_1);
}

#[test]
fn sidecar_hand_built_tree_and_leaf_tags() {
    let node = sample_sidecar().to_node();
    let SimpleJsonNode::Class(top) = &node else {
        panic!("root must be a class")
    };
    let keys: Vec<_> = top.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        [
            "95000_Setting",
            "20000_Event",
            "40000_Common",
            "30000_MiniGame"
        ]
    );
    // gil > i32::MAX falls to the Double tag; status 5e9 likewise; booleans are strings.
    let text = format!("{node:?}");
    assert!(text.contains("Double(3000000000.0)"));
    assert!(text.contains("String(\"True\")"));
    // Hand-modified blob with an extra key must be rejected.
    let mut extra = top.clone();
    extra.push(("MetaDataSlotID".to_owned(), SimpleJsonNode::Int(1)));
    let bytes = tantalus_legacy::SimpleJsonDocument::encode(&SimpleJsonNode::Class(extra)).unwrap();
    assert!(MemoriaExtra::decode(&bytes).is_err());
    // Truncation is an error, not a partial tree.
    let blob = sample_sidecar().encode().unwrap();
    assert!(MemoriaExtra::decode(&blob[..blob.len() - 1]).is_err());
}
