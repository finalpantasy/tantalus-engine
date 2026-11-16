//! Hand-derived cases for the Chocograph and chocobo C# rules. Each test
//! names the Memoria function (pin `c716e1fa`, `Assembly-CSharp/`) it checks;
//! expected values are worked out in the comments.

use tantalus_legacy::chocobo::*;
use tantalus_legacy::saves::ff9_tree::EventGlobal;

/// found: bytes 187..=189 = 05 00 80 -> 0x800005 (bits 0, 2, 23).
/// opened = bytes 184..=186 = 01 00 00 -> bit 0. ability (byte 191) = 3.
fn sample() -> EventGlobal {
    let mut g = EventGlobal::zeroed();
    let b = g.as_bytes_mut();
    b[187] = 0x05;
    b[189] = 0x80;
    b[184] = 0x01;
    b[191] = 3;
    g
}

#[test]
fn requirement_table_matches_source_blocks() {
    // ChocographUI.cs:480-506: 6 Field, 6 Reef, 4 Mountain, 4 Sea, 4 Sky.
    let count = |icon| REQUIREMENT.iter().filter(|i| **i == icon).count();
    assert_eq!(count(Icon::Field), 6);
    assert_eq!(count(Icon::Reef), 6);
    assert_eq!(count(Icon::Mountain), 4);
    assert_eq!(count(Icon::Sea), 4);
    assert_eq!(count(Icon::Sky), 4);
    assert_eq!(REQUIREMENT[5], Icon::Field);
    assert_eq!(REQUIREMENT[6], Icon::Reef);
    assert_eq!(REQUIREMENT[12], Icon::Mountain);
    assert_eq!(REQUIREMENT[16], Icon::Sea);
    assert_eq!(REQUIREMENT[20], Icon::Sky);
}

#[test]
fn get_event_data_reads_found_opened_ability() {
    // GetEventData (ChocographUI.cs:238-262): found 0x800005 -> 3 maps, opened 1 -> 1 gem.
    let book = ChocographBook::from_event(&sample(), 0);
    assert_eq!(book.map_count(), 3);
    assert_eq!(book.gem_count(), 1);
    assert_eq!(book.ability(), 3);
    assert!(book.has_map(0) && book.has_map(2) && book.has_map(23));
    assert!(!book.has_map(1));
    assert!(book.has_gem(0) && !book.has_gem(2));
}

#[test]
fn list_rows_follow_display_chocograph_list() {
    // DisplayChocographList (ChocographUI.cs:285-307), ability 3.
    let book = ChocographBook::from_event(&sample(), 0);
    // 0: map, gem, Field(0) < 3 reachable -> BoxOpen, help 34 + 0.
    let e0 = book.entry(0).unwrap();
    assert_eq!(e0.box_icon, Some(Icon::BoxOpen));
    assert_eq!((e0.name_text, e0.help_text, e0.detail_text), (10, 34, 58));
    assert_eq!(e0.map_sprite, "chocograph_map_00");
    // 2: map, no gem, reachable -> BoxClose, help 34 + 2.
    let e2 = book.entry(2).unwrap();
    assert_eq!(e2.box_icon, Some(Icon::BoxClose));
    assert_eq!(e2.help_text, 36);
    // 23: map, Sky(4), 3 > 4 false -> BoxDisableClose, help text 5.
    let e23 = book.entry(23).unwrap();
    assert!(!e23.can_reach);
    assert_eq!(e23.box_icon, Some(Icon::BoxDisableClose));
    assert_eq!(e23.help_text, 5);
    assert_eq!(e23.map_sprite, "chocograph_map_23");
    // 1: no map -> hidden row.
    let e1 = book.entry(1).unwrap();
    assert!(!e1.has_map);
    assert_eq!(e1.box_icon, None);
    assert!(book.entry(24).is_none());
}

#[test]
fn disabled_open_box_when_gem_taken_but_out_of_reach() {
    // Same ability check with the gem bit set: BoxDisableOpen (ChocographUI.cs:298).
    let mut g = sample();
    g.as_bytes_mut()[186] = 0x80; // opened bit 23
    let book = ChocographBook::from_event(&g, 0);
    assert_eq!(book.entry(23).unwrap().box_icon, Some(Icon::BoxDisableOpen));
}

#[test]
fn select_and_cancel_follow_on_key_confirm() {
    // OnKeyConfirm (ChocographUI.cs:142-160) and Cancel (:123-133).
    let mut book = ChocographBook::from_event(&sample(), 0);
    assert_eq!(book.select(1), Err(SelectError::NoMap));
    assert_eq!(book.select(23), Err(SelectError::AbilityTooLow));
    assert_eq!(book.select(24), Err(SelectError::OutOfRange));
    assert_eq!((book.selected, book.hintmap_id), (-1, 0));
    book.select(2).unwrap();
    assert_eq!((book.selected, book.hintmap_id), (2, 3));
    assert!(book.cancel_selection());
    assert_eq!((book.selected, book.hintmap_id), (-1, 0));
    assert!(!book.cancel_selection());
}

#[test]
fn update_equipped_hint_map_clamps() {
    // UpdateEquipedHintMap (ChocographUI.cs:225-236).
    let at = |h| ChocographBook::from_event(&sample(), h);
    assert_eq!((at(0).selected, at(0).hintmap_id), (-1, 0));
    assert_eq!((at(5).selected, at(5).hintmap_id), (4, 5));
    assert_eq!((at(24).selected, at(24).hintmap_id), (23, 24));
    // Above HintMapMax (24): reset.
    assert_eq!((at(25).selected, at(25).hintmap_id), (-1, 0));
    // Negative: Math.Max(h, 0) - 1 = -1, hintmap_id left alone.
    assert_eq!((at(-3).selected, at(-3).hintmap_id), (-1, -3));
}

#[test]
fn ability_and_requirement_icons() {
    // DisplayChocoboAbilityInfo (:264-268): ability 3 -> Field, Reef, Mountain.
    let book = ChocographBook::from_event(&sample(), 0);
    assert_eq!(
        book.ability_icons(),
        Some(vec![Icon::Field, Icon::Reef, Icon::Mountain])
    );
    // DisplayHint (:340-345), picture 14 is Mountain (level 2), ability 3:
    // k = 0,1,2 all reached.
    assert_eq!(
        book.required_icons(14),
        Some(vec![Icon::Field, Icon::Reef, Icon::Mountain])
    );
    // Picture 20 is Sky (level 4): k = 3 and 4 are not reached (3 <= 3, 3 <= 4) -> Unknown.
    assert_eq!(
        book.required_icons(20),
        Some(vec![
            Icon::Field,
            Icon::Reef,
            Icon::Mountain,
            Icon::Unknown,
            Icon::Unknown
        ])
    );
    // Ability 6 would overflow the 5-slot array in the source.
    let mut g = sample();
    g.as_bytes_mut()[191] = 6;
    assert_eq!(ChocographBook::from_event(&g, 0).ability_icons(), None);
    assert_eq!(Icon::Unknown.sprite_name(), "chocograph_icon_unknown_null");
    assert_eq!(Icon::Mountain.sprite_name(), "chocograph_icon_mt");
}

#[test]
fn all_treasure_cheat_and_tracker() {
    // AllTreasureCheating (EMinigame.cs:449-455): opened 0xFFFFFE, found 0xFFFFFF, ability 5.
    let mut g = EventGlobal::zeroed();
    all_treasure_cheat(&mut g);
    assert_eq!(g.chocograph_opened(), 0xFF_FFFE);
    assert_eq!(g.chocograph_found(), 0xFF_FFFF);
    assert_eq!(g.chocograph_ability(), 5);
    let book = ChocographBook::from_event(&g, 0);
    assert_eq!((book.map_count(), book.gem_count()), (24, 23));
    // AllTreasureAchievement (:430-440): fires once on the change to 0xFFFFFF.
    let mut tracker = TreasureTracker::new(&g);
    assert_eq!(tracker.update(&g), None);
    g.set_chocograph_opened(0xFF_FFFF);
    assert_eq!(tracker.update(&g), Some(24));
    assert_eq!(tracker.update(&g), None);
    // Starting already complete never fires (Initialize copies the flags).
    let mut done = TreasureTracker::new(&g);
    assert_eq!(done.update(&g), None);
}

#[test]
fn treasure_hunter_points_and_rank() {
    // GetTreasureHunterPoints (EventState.cs:53-70): byte 896 = 0xFF -> 8, byte 970 = 0x01 -> 1,
    // byte 182 = 0x03 -> 2 bits * 2 = 4. Total 13. Ranges per source: 896..=960, 966..=975, 182..=186.
    let mut g = EventGlobal::zeroed();
    let b = g.as_bytes_mut();
    b[896] = 0xFF;
    b[970] = 0x01;
    b[182] = 0x03;
    b[961] = 0xFF; // outside both ranges: not counted
    assert_eq!(g.treasure_hunter_points(), 13);
    assert_eq!(treasure_hunter_rank(13), 'H');
    // Comment table, EventState.cs:55-63.
    assert_eq!(treasure_hunter_rank(199), 'H');
    assert_eq!(treasure_hunter_rank(200), 'G');
    assert_eq!(treasure_hunter_rank(239), 'F');
    assert_eq!(treasure_hunter_rank(259), 'E');
    assert_eq!(treasure_hunter_rank(279), 'D');
    assert_eq!(treasure_hunter_rank(299), 'C');
    assert_eq!(treasure_hunter_rank(319), 'B');
    assert_eq!(treasure_hunter_rank(339), 'A');
    assert_eq!(treasure_hunter_rank(340), 'S');
}

#[test]
fn gysahl_greens_gate() {
    // FF9FItem_Vegetable (ItemUI.cs:958-969).
    let mut g = EventGlobal::zeroed();
    g.as_bytes_mut()[191] = 1;
    // Wrong mode, off track, not the player: refused, byte 181 untouched.
    assert!(!use_gysahl_greens(1, &mut g, true, 18));
    assert!(!use_gysahl_greens(3, &mut g, true, 4));
    assert!(!use_gysahl_greens(3, &mut g, false, 18));
    assert_eq!(g.as_bytes()[181], 0);
    // Every listed track works.
    for topo in [3, 18, 21, 22, 28] {
        let mut g2 = g.clone();
        assert!(
            use_gysahl_greens(3, &mut g2, true, topo),
            "topograph {topo}"
        );
        assert_eq!(g2.as_bytes()[181], 1);
    }
    assert!(use_gysahl_greens(3, &mut g, true, 18));
    assert_eq!(g.as_bytes()[181], 1);
    // Second use is refused (181 != 0).
    assert!(!use_gysahl_greens(3, &mut g, true, 18));
    // No chocobo kind (byte 191 == 0).
    let mut none = EventGlobal::zeroed();
    assert!(!use_gysahl_greens(3, &mut none, true, 18));
}

#[test]
fn world_predicates() {
    // EventCollision.cs:277-290.
    assert!(!is_riding_chocobo(0));
    assert!(is_riding_chocobo(1) && is_riding_chocobo(6));
    assert!(!is_riding_chocobo(7));
    assert!(is_walking_or_flying_in_forest(5, 7, 36));
    assert!(is_walking_or_flying_in_forest(6, 7, 38));
    assert!(!is_walking_or_flying_in_forest(4, 7, 36));
    assert!(!is_walking_or_flying_in_forest(5, 6, 36));
    assert!(!is_walking_or_flying_in_forest(5, 7, 39));
    assert!(is_walking_in_forest(5, 37));
    assert!(!is_walking_in_forest(6, 37));
    assert!(!is_walking_in_forest(5, 35));
    // ETb.FixChocoAccidenlyFly (ETb.cs:458-462).
    assert!(forest_dialog_disables_collider(3, true, 54));
    assert!(forest_dialog_disables_collider(3, true, 62));
    assert!(!forest_dialog_disables_collider(3, true, 63));
    assert!(!forest_dialog_disables_collider(1, true, 54));
    assert!(!forest_dialog_disables_collider(3, false, 54));
    // Obj.cs:491-495 and BeeChocobo.SetType.
    assert_eq!(ChocoboKind::from_obj_index(3), Some(ChocoboKind::Yellow));
    assert_eq!(ChocoboKind::from_obj_index(7), Some(ChocoboKind::Gold));
    assert_eq!(ChocoboKind::from_obj_index(2), None);
    assert_eq!(ChocoboKind::from_obj_index(8), None);
    assert_eq!(ChocoboKind::Blue.obj_index(), 6);
}

#[test]
fn hot_and_cold_hud_switches() {
    // EventHUD.cs:384-395.
    assert_eq!(forest_hud(2950, true, 5.0, true), ForestHud::OpenHot);
    assert_eq!(forest_hud(2952, true, 0.0, true), ForestHud::CloseHot);
    assert_eq!(forest_hud(2951, false, 9.0, true), ForestHud::CloseHot);
    assert_eq!(forest_hud(2951, false, 9.0, false), ForestHud::Nothing);
    assert_eq!(forest_hud(2953, true, 5.0, true), ForestHud::Nothing);
    // EventHUD.cs:183-189.
    assert_eq!(
        special_hud_for_message(945, 34),
        Some(MinigameHud::ChocoHotInstruction)
    );
    assert_eq!(
        special_hud_for_message(945, 35),
        Some(MinigameHud::ChocoHotInstruction)
    );
    assert_eq!(special_hud_for_message(945, 36), None);
    assert_eq!(special_hud_for_message(946, 34), None);
}

#[test]
fn beak_achievement_and_fat_chocobo() {
    // ChocoboBeakLV99Achievement (EMinigame.cs:261-285), beak level in Global Byte 139.
    let mut g = EventGlobal::zeroed();
    g.as_bytes_mut()[139] = 57;
    assert_eq!(beak_achievement_level(2950, "US", 320, &g), Some(57));
    assert_eq!(beak_achievement_level(2951, "UK", 277, &g), Some(57));
    assert_eq!(beak_achievement_level(2952, "JP", 325, &g), Some(57));
    assert_eq!(beak_achievement_level(2952, "FR", 326, &g), Some(57));
    assert_eq!(beak_achievement_level(2950, "FR", 278, &g), Some(57));
    // US 326 is the "other language" id, not US.
    assert_eq!(beak_achievement_level(2950, "US", 326, &g), None);
    assert_eq!(beak_achievement_level(2953, "US", 320, &g), None);
    // SetFatChocoboId (EMinigame.cs:362-366).
    assert!(fat_chocobo_opponent(2955, 9));
    assert!(!fat_chocobo_opponent(2955, 8));
    assert!(!fat_chocobo_opponent(2954, 9));
}

#[test]
fn chocobo_check_tracks() {
    // w_frameChocoboCheck (ff9.cs:4353-4370).
    for topo in [3, 18, 21, 22, 28] {
        assert!(chocobo_check(true, topo));
        assert!(!chocobo_check(false, topo));
    }
    assert!(!chocobo_check(true, 0));
    assert!(!chocobo_check(true, 4));
}
