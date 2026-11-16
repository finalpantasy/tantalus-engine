//! Scene -> reward-row adapter: real `SB2` monster fields land on the legacy
//! `EnemyRewardData` shape the reward path consumes.

use tantalus_assets::pc::battle_scene::{
    BattleScene, ByteOrTokens, MonsterStats, MonsterType, Pattern, Placement, SceneHeader,
};
use tantalus_legacy::battle::rewards::{NO_CARD, NO_ITEM};
use tantalus_tools::battle_scene_rewards::{pattern_bonus, pattern_rewards, reward_totals};

fn monster(gil: u32, exp: u32, items: [u32; 4], card: i32) -> MonsterType {
    MonsterType {
        resist_status: 0,
        auto_status: 0,
        initial_status: 0,
        max_hp: 1000,
        max_mp: 100,
        win_gil: gil,
        win_exp: exp,
        win_items: items,
        steal_items: [NO_ITEM; 4],
        win_item_rates: [256, 96, 32, 1],
        steal_item_rates: [256, 64, 16, 1],
        win_card_rate: 32,
        radius: 0,
        geo: 0,
        mot: [0; 6],
        mesh: [0; 2],
        flags: 0,
        ap: 0,
        stats: MonsterStats {
            speed: 0,
            strength: 0,
            magic: 0,
            spirit: 0,
            pad: 0,
            trans: 0,
            cur_capa: 0,
            max_capa: 0,
        },
        guard_element: ByteOrTokens::Byte(0),
        absorb_element: ByteOrTokens::Byte(0),
        half_element: ByteOrTokens::Byte(0),
        weak_element: ByteOrTokens::Byte(0),
        level: 10,
        category: ByteOrTokens::Byte(0),
        hit_rate: 0,
        physical_defence: 0,
        physical_evade: 0,
        magical_defence: 0,
        magical_evade: 0,
        blue_magic: 0,
        bone: [0; 4],
        die_sfx: 0,
        konran: 0,
        mes_cnt: 0,
        icon_bone: [0; 6],
        icon_y: [0; 6],
        icon_z: [0; 6],
        start_sfx: 0,
        shadow_x: 0,
        shadow_z: 0,
        shadow_bone: 0,
        win_card: card,
        shadow_ofs_x: 0,
        shadow_ofs_z: 0,
        shadow_bone2: 0,
        pad0: 0,
        pad1: 0,
        pad2: 0,
    }
}

fn scene() -> BattleScene {
    BattleScene {
        header: SceneHeader {
            version: 10,
            pattern_count: 1,
            type_count: 2,
            attack_count: 0,
            flags: 0,
            pad1: 0,
            battle_map_index: Some(0),
        },
        patterns: vec![Pattern {
            rate: 100,
            monster_count: 2,
            camera: 0,
            pad0: 0,
            ap: 3,
            monsters: [
                Placement {
                    type_no: 0,
                    flags: Placement::FLG_TARGETABLE,
                    pease: 0,
                    pad: 0,
                    x: 0,
                    y: 0,
                    z: 0,
                    rot: 0,
                },
                Placement {
                    type_no: 1,
                    flags: Placement::FLG_TARGETABLE,
                    pease: 0,
                    pad: 0,
                    x: 100,
                    y: 0,
                    z: 0,
                    rot: 0,
                },
                Placement {
                    type_no: 0,
                    flags: 0,
                    pease: 0,
                    pad: 0,
                    x: 0,
                    y: 0,
                    z: 0,
                    rot: 0,
                },
                Placement {
                    type_no: 0,
                    flags: 0,
                    pease: 0,
                    pad: 0,
                    x: 0,
                    y: 0,
                    z: 0,
                    rot: 0,
                },
            ],
        }],
        monster_types: vec![
            monster(200, 240, [12, NO_ITEM, NO_ITEM, NO_ITEM], -1),
            monster(100, 60, [13, 14, NO_ITEM, NO_ITEM], 42),
        ],
        attacks: vec![],
        trailing_bytes: 0,
        trailing_all_zero: true,
        file_len: 0,
        trailing: vec![],
    }
}

#[test]
fn pattern_rewards_map_scene_fields_onto_enemy_rows() {
    let scene = scene();
    let rows = pattern_rewards(&scene, 0).unwrap();
    assert_eq!(rows.len(), 2, "only the two used placements become rows");
    assert_eq!(rows[0].type_id, 0);
    assert_eq!((rows[0].gil, rows[0].exp), (200, 240));
    assert_eq!(rows[0].items[0], 12);
    assert_eq!(rows[0].item_rates, [256, 96, 32, 1]);
    assert_eq!(rows[0].card, NO_CARD, "win_card -1 is no card");
    assert_eq!(rows[1].type_id, 1);
    assert_eq!(rows[1].card, 42, "win_card id carries over");
    assert_eq!(rows[1].card_rate, 32);
    assert_eq!(reward_totals(&rows), (300, 300));

    // Pattern AP and participation build the bonus the rows pay into.
    let bonus = pattern_bonus(&scene, 0, 0b0011, false).unwrap();
    assert_eq!(bonus.ap, 3);
    assert_eq!(bonus.member_flag, 0b0011);
    assert!(!bonus.event);
    assert_eq!(bonus.gil, 0, "zeroed until enemy deaths");

    assert!(pattern_rewards(&scene, 9).is_none(), "no such pattern");
}

#[test]
fn missing_monster_type_invalidates_the_pattern() {
    let mut scene = scene();
    scene.patterns[0].monsters[1].type_no = 7;
    assert!(pattern_rewards(&scene, 0).is_none());
    assert!(pattern_bonus(&scene, 0, 0b0001, false).is_none());
}
