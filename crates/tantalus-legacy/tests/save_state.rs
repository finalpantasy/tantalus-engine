//! Tests for the typed G8 save state. Synthetic data only: no real FF9 save is read anywhere.

use tantalus_game::new_game::{new_game_character_stats, parse_base_stats, parse_leveling};
use tantalus_legacy::saves::ff9_tree::{CardSlot, Ff9SaveTree, KeyOrder, Resources};
use tantalus_legacy::saves::state::{ItemStack, SaveState};

fn populated() -> SaveState {
    let mut s = SaveState::empty();
    s.mode.mode = 5;
    s.mode.prev_mode = 3;
    s.mode.timer_seconds = 1234.0;
    s.mode.timer_control = true;
    s.field.fld_map_no = 1234;
    s.field.fld_loc_no = -2;
    s.field.btl_map_no = 77;
    s.field.btl_sub_map_no = -3;
    s.world.wld_map_no = 9;
    s.world.wld_loc_no = 10;
    s.world.camera.rotation = 1.5;
    s.world.camera.rotation_max = 2.5;
    s.world.camera.rotation_rev = 3.5;
    s.world.camera.upper_counter = -7;
    s.world.camera.upper_counter_speed = 11;
    s.world.camera.upper_counter_force = true;
    s.world.camera.hintmap = 0xDEAD_BEEF;
    s.variables.set_scenario_counter(9860);
    s.variables.set_field_entrance(0x1234);
    s.variables.as_bytes_mut()[900] = 0xFF;
    s.variables.as_bytes_mut()[2047] = 0x5A;
    s.step_count = 424_242;
    s.play_time_seconds = 98_765.432_1;
    s.auto_save_bgm_id = 42;
    s.party.members = [Some(0), None, Some(3), Some(8)];
    s.party.gil = 1_234_567;
    s.party.escape_no = 4;
    s.party.summon_flag = 0x8001;
    s.party.frog_no = -5;
    s.party.steal_no = 6;
    s.party.dragon_no = 7;
    s.party.items[0] = Some(ItemStack { id: 0, count: 99 });
    s.party.items[5] = Some(ItemStack { id: 17, count: 3 });
    s.party.rare_item_flags[0] = 0b0101;
    s.party.rare_item_flags[63] = 0x80;
    for (i, c) in s.party.characters.iter_mut().enumerate() {
        let n = i as u8 + 1;
        c.name = format!("Synthetic {n}");
        c.category = n;
        c.level = n * 3;
        c.exp = 1000 * u32::from(n);
        c.cur = Resources {
            hp: 100 * u16::from(n),
            mp: -i16::from(n),
            at: 5,
            at_coef: -4,
            capa: n,
        };
        c.max = Resources {
            hp: 9999,
            mp: 999,
            at: 1,
            at_coef: 2,
            capa: 99,
        };
        c.equip = [n, n + 1, n + 2, n + 3, n + 4];
        c.status = 0x40 | n;
        c.trance = n;
        c.web_bone = n;
        c.ability_ap = (0..48).map(|k| k as u8 ^ n).collect();
        c.support_abilities = [0xA000_0000 | u32::from(n), u32::from(n) << 8];
        c.info.slot_no = n;
        c.info.serial_no = n + 10;
        c.info.row = 1;
        c.info.win_pose = 2;
        c.info.party = 1;
        c.info.menu_type = 3;
        c.basis.max_hp = 1000 + i16::from(n);
        c.basis.max_mp = 100 + i16::from(n);
        c.basis.stats = [n, n, n, n];
        c.basis.bonus = [u16::from(n) * 300; 4];
        c.basis.elem = [1, 2, 3, 4];
        c.basis.defence = [5, 6, 7, 8];
        c.bonus_cap = 10 * u32::from(n);
    }
    s.cards.win = 12;
    s.cards.lose = -1;
    s.cards.draw = 3;
    s.cards.cards[0] = Some(CardSlot {
        id: 7,
        side: 0,
        atk: 4,
        card_type: 2,
        pdef: 1,
        mdef: 2,
        cpoint: 3,
        arrow: 0xA5,
    });
    // Unmodeled data must survive a save and load untouched.
    s.carried.other_99000[0] = 0x1234_5678;
    s.carried.other_99000[383] = -9;
    s.carried.reserved_91000[5] = 77;
    s.carried.achievement.synthesis_count = 31;
    s.carried.setting.time = 3.25;
    s
}

#[test]
fn state_round_trips_through_tree_and_payload_bytes() {
    let state = populated();
    let tree = state.to_tree().unwrap();
    assert_eq!(SaveState::from_tree(&tree), state);
    let bytes = state.encode().unwrap();
    assert_eq!(bytes.len(), Ff9SaveTree::PAYLOAD_BYTES);
    let back = SaveState::decode(&bytes).unwrap();
    assert_eq!(back, state);
    assert_eq!(back.scenario_counter(), 9860);
    assert_eq!(back.field_entrance(), 0x1234);
    assert_eq!(back.variables.as_bytes()[2047], 0x5A);
}

#[test]
fn tree_to_state_to_tree_is_identity() {
    let tree = populated().to_tree().unwrap();
    assert_eq!(SaveState::from_tree(&tree).to_tree().unwrap(), tree);
    let blank = Ff9SaveTree::blank();
    assert_eq!(SaveState::from_tree(&blank).to_tree().unwrap(), blank);
}

#[test]
fn encoded_bytes_equal_direct_tree_encoding() {
    let state = populated();
    assert_eq!(
        state.encode().unwrap(),
        state
            .to_tree()
            .unwrap()
            .encode_payload(KeyOrder::Ordinal)
            .unwrap()
    );
}

#[test]
fn save_is_deterministic_and_hash_survives_load() {
    let state = populated();
    let a = state.encode().unwrap();
    let b = state.clone().encode().unwrap();
    assert_eq!(a, b);
    let loaded = SaveState::decode(&a).unwrap();
    assert_eq!(state.state_hash().unwrap(), loaded.state_hash().unwrap());
    // Saving the loaded state again gives the same bytes.
    assert_eq!(loaded.encode().unwrap(), a);
    // A single changed byte changes the hash.
    let mut other = loaded.clone();
    other.variables.as_bytes_mut()[1000] ^= 1;
    assert_ne!(state.state_hash().unwrap(), other.state_hash().unwrap());
    let mut gil = loaded;
    gil.party.gil += 1;
    assert_ne!(state.state_hash().unwrap(), gil.state_hash().unwrap());
}

#[test]
fn empty_slots_use_the_writer_markers() {
    let tree = SaveState::empty().to_tree().unwrap();
    assert_eq!(tree.common.slot, [255; 4]);
    assert!(tree.common.items.iter().all(|&i| i == (255, 0)));
    assert!(tree.mini_game.cards.iter().all(|&c| c == CardSlot::EMPTY));
    // An item with id 255 but a count is a real stack, not an empty slot, and survives.
    let mut s = SaveState::empty();
    s.party.items[2] = Some(ItemStack { id: 255, count: 4 });
    s.party.items[3] = Some(ItemStack { id: 9, count: 0 });
    assert_eq!(SaveState::decode(&s.encode().unwrap()).unwrap(), s);
}

#[test]
fn wrong_collection_lengths_are_rejected_not_padded() {
    let mut s = SaveState::empty();
    s.party.characters.pop();
    assert!(s.to_tree().is_err());
    let mut s = SaveState::empty();
    s.party.items.pop();
    assert!(s.encode().is_err());
    let mut s = SaveState::empty();
    s.cards.cards.push(None);
    assert!(s.encode().is_err());
    assert!(SaveState::decode(&[0u8; 10]).is_err());
}

#[test]
fn unmodeled_report_names_what_is_carried() {
    let report = SaveState::empty().unmodeled_report();
    let names: Vec<_> = report.iter().map(|e| e.name).collect();
    assert!(names.contains(&"50000_Setting"));
    assert!(names.contains(&"99000_Other"));
    let other = report.iter().find(|e| e.name == "99000_Other").unwrap();
    assert_eq!(other.bytes, 1536);
    assert!(report.iter().all(|e| !e.reason.is_empty()));
}

fn new_game_csvs() -> (String, String) {
    // Synthetic rows in the reference layout (not game data).
    let mut base = String::new();
    for id in 0..12 {
        base.push_str(&format!("Name{id};{id};10;{};{};10;5\n", 20 + id, 10 + id));
    }
    let mut levels = String::new();
    for level in 1..=99 {
        levels.push_str(&format!("{};{};50\n", level * 10, 100 + level));
    }
    (base, levels)
}

#[test]
fn new_game_then_save_then_load() {
    let (base_csv, level_csv) = new_game_csvs();
    let base = parse_base_stats(&base_csv).unwrap();
    let levels = parse_leveling(&level_csv).unwrap();
    let stats = new_game_character_stats(&base, &levels);

    // Plumbing only: character id i goes to saved slot i for ids 0..9. The real
    // `SelectOldSaveSlot` remapping is not implemented, so slot names are not claimed.
    let mut state = SaveState::empty();
    for (i, ch) in state.party.characters.iter_mut().enumerate() {
        let s = stats[&(i as i32)];
        ch.name = format!("New{i}");
        ch.level = 1;
        ch.cur.hp = s.current_hp as u16;
        ch.max.hp = s.max_hp as u16;
        ch.cur.mp = s.current_mp as i16;
        ch.max.mp = s.max_mp as i16;
        ch.info.slot_no = i as u8;
    }
    state.party.members = [Some(0), None, None, None];
    state.party.gil = 0;
    let before_hash = state.state_hash().unwrap();

    let loaded = SaveState::decode(&state.encode().unwrap()).unwrap();
    assert_eq!(loaded, state);
    assert_eq!(loaded.state_hash().unwrap(), before_hash);
    // Character 1 from the reference formula: str 21 -> 101 * 21 / 50 = 42, mgc 11 -> 5.
    let c1 = &loaded.party.characters[1];
    assert_eq!((c1.cur.hp, c1.max.hp, c1.cur.mp, c1.max.mp), (42, 42, 5, 5));
    assert_eq!(loaded.party.members, [Some(0), None, None, None]);
    // A new game starts every record at its maximum.
    assert!(
        loaded
            .party
            .characters
            .iter()
            .all(|c| c.cur.hp == c.max.hp && c.cur.mp == c.max.mp)
    );
}
