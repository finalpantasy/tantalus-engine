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

#[test]
fn pattern_machine_units_build_enemy_units_from_monster_types() {
    use tantalus_tools::battle_scene_rewards::pattern_machine_units;

    let mut scene = scene();
    // Type 0: guard element 3, category Undead|Beast (2 | 16), resist bit 40.
    scene.monster_types[0].guard_element = ByteOrTokens::Byte(3);
    scene.monster_types[0].category = ByteOrTokens::Byte(18);
    scene.monster_types[0].resist_status = 1 << 40;
    scene.monster_types[0].stats.spirit = 15;
    scene.monster_types[0].stats.speed = 12;
    let units = pattern_machine_units(&scene, 0).unwrap();
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].id, 16);
    assert_eq!(units[1].id, 32);
    assert_eq!(units[0].max_hp, 1000);
    assert!(!units[0].is_player);
    assert_eq!(units[0].will, 15, "spirit feeds the calculator's Will");
    assert_eq!(units[0].dexterity, 12);
    assert_eq!(units[0].guard_elements.0, 3);
    assert!(units[0].is_undead, "category bit 16 is Undead");
    assert!(!units[0].is_flight);
    assert_eq!(units[0].status_resist, 1 << 40);
    // Type 1 (default category byte 0): neither undead nor flight.
    assert!(!units[1].is_undead && !units[1].is_flight);

    assert!(pattern_machine_units(&scene, 9).is_none());
    scene.patterns[0].monsters[1].type_no = 7;
    assert!(pattern_machine_units(&scene, 0).is_none());
}

#[test]
fn assemble_legacy_battle_runs_a_scene_battle_to_victory() {
    use std::collections::BTreeMap;
    use tantalus_contracts::{ContentId, EntityId, Tick};
    use tantalus_game::FieldWorld;
    use tantalus_game::battle::{BattleCommand, EncounterCoordinator, GameServices};
    use tantalus_legacy::battle::machine::{ABILITY_ATTACK, BattleConfig};
    use tantalus_legacy::battle::queue::cmd;
    use tantalus_legacy::battle::rewards::{BattleBonus, BattleReward, PartyMember, PartySnapshot};
    use tantalus_legacy::battle::rng::{Draw, SequenceRandom};
    use tantalus_legacy::battle::{BattleUnit, StatusDataRow, StatusTable, WeaponStats};
    use tantalus_tools::battle_scene_rewards::assemble_legacy_battle;

    let mut scene = scene();
    // Fightable enemies: low HP, nonzero spirit (the calculator's Will).
    for monster in &mut scene.monster_types {
        monster.max_hp = 60;
        monster.stats.spirit = 10;
    }
    let mut player = BattleUnit::new(1, true);
    player.level = 20;
    player.cur_hp = 500;
    player.max_hp = 500;
    player.strength = 30;
    player.dexterity = 30;
    player.will = 20;
    player.weapon = Some(WeaponStats {
        power: 25,
        rate: 0,
        elements: tantalus_legacy::battle::Elements::NONE,
        category: WeaponStats::SHORT_RANGE,
        status: 0,
    });
    let player_entity = EntityId::new(101);
    let attack = ContentId::parse("fixture:action/attack").unwrap();
    let setup = assemble_legacy_battle(
        &scene,
        0,
        &BattleConfig::default(),
        &StatusTable::new(vec![StatusDataRow::default(); 32]),
        &[(player_entity, player)],
        BTreeMap::from([(attack.clone(), (cmd::ATTACK, ABILITY_ATTACK))]),
    )
    .unwrap();
    assert_eq!(setup.reward_rows.len(), 2);
    assert_eq!(setup.pattern_ap, 3);
    assert_eq!(
        setup.enemy_entities,
        vec![EntityId::new(16), EntityId::new(32)]
    );

    let mut coordinator = EncounterCoordinator::new();
    coordinator
        .begin(
            tantalus_game::field::world::EncounterRequest {
                rush: 0,
                group: None,
                battle: 0x55,
                steiner_state: false,
            },
            setup.adapter,
        )
        .unwrap();
    let mut world = FieldWorld::new();
    world.gil = 10;

    // Death-driven rewards straight from the assembled rows.
    let mut bonus = BattleBonus::new(setup.pattern_ap, 0b0001, false);
    let mut rows = setup.reward_rows.clone();
    let mut rng = SequenceRandom::with_kinds(&[
        (Draw::Random8, 0),
        (Draw::Random8, 0),
        (Draw::Random8, 0),
        (Draw::Random8, 0),
    ]);
    let mut deaths = 0usize;

    let mut outcome = None;
    for tick in 0..6000u64 {
        let mut services = tantalus_game::battle::WorldGameServices::new(&mut world);
        let mut input = tantalus_game::battle::CombatInput::new(Tick::new(tick));
        input.command = Some(BattleCommand {
            actor: player_entity,
            action: attack.clone(),
            targets: setup.enemy_entities.clone(),
        });
        if let Some(done) = coordinator.advance(&input, &mut services).unwrap() {
            outcome = Some(done);
        }
        drop(services);
        if let Some(tantalus_game::battle::AnyRuleset::Legacy(adapter)) =
            coordinator.active_ruleset_mut()
        {
            for event in adapter.take_events() {
                if let tantalus_game::battle::CombatEvent::UnitDefeated { .. } = event {
                    assert!(bonus.set_bonus(&mut rows, deaths, &mut rng));
                    deaths += 1;
                }
            }
        }
        if outcome.is_some() {
            break;
        }
    }
    assert_eq!(outcome, Some(tantalus_game::battle::BattleOutcome::Victory));
    assert_eq!(deaths, 2, "both scene enemies died exactly once");
    assert_eq!(bonus.gil, 300, "scene-declared spoils accumulated");

    // Pay the field world through the reference split.
    let mut member = PartyMember {
        in_battle: true,
        cur_hp: 500,
        cur_mp: 100,
        rewards_disabled: false,
        level: 1,
        exp: 0,
        base: tantalus_legacy::battle::rewards::BaseStats {
            dex: 23,
            str_: 21,
            mgc: 18,
            wpr: 23,
            gems: 18,
        },
        bonus: Default::default(),
        basis: [23, 21, 18, 23],
        max_hp: 500,
        max_mp: 100,
        max_capa: 18,
        cur_capa: 18,
        elem: [23, 21, 18, 23],
        equip_stat_bonus: [0; 4],
        equipped_abilities: vec![],
        learnable: vec![],
        pa: vec![],
    };
    member.learnable = vec![tantalus_legacy::battle::rewards::PaEntry { id: 7, max_ap: 2 }];
    member.pa = vec![0];
    let party = PartySnapshot {
        gil: 10,
        members: [Some(member), None, None, None],
        items: vec![],
        cards: vec![],
    };
    let reward = BattleReward::from_bonus(&bonus, &party);
    let bundle = tantalus_game::battle::bridge::bundle_from_battle_reward(
        &reward,
        [Some(0), None, None, None],
        1,
    );
    {
        let mut services = tantalus_game::battle::WorldGameServices::new(&mut world);
        services.commit_rewards(&bundle).unwrap();
    }
    assert_eq!(world.gil, 310);
    assert_eq!(
        world.character_exp[&0], 300,
        "300 EXP over the single eligible line"
    );
}
