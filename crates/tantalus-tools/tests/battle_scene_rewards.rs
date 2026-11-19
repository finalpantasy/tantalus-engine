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
                timer: None,
                pattern_roll: 0,
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

#[test]
fn scene_id_table_resolves_encounter_ids_to_event_assets() {
    use tantalus_tools::battle_scene_rewards::{event_asset_name, parse_scene_id_table};

    let tsv = "\
# comment line
id\tscene_key
66\tBSC_AC_E028F
85\tBSC_FR_R015
191\tBSC_AB_E039
";
    let table = parse_scene_id_table(tsv);
    assert_eq!(table.len(), 3);
    assert_eq!(table[&66], "BSC_AC_E028F");
    assert_eq!(table.get(&67), None);
    assert_eq!(event_asset_name(&table[&85]).unwrap(), "EVT_BATTLE_FR_R015");
    assert_eq!(event_asset_name("NO_PREFIX"), None);

    // The checked-in audit table parses with the same helper.
    let audit = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../audit/battle-scene-ids.tsv"
    ))
    .unwrap();
    let full = parse_scene_id_table(&audit);
    assert_eq!(full.len(), 856, "pinned SceneData row count");
    assert_eq!(full[&66], "BSC_AC_E028F");
}

#[test]
fn run_scene_battle_drives_a_full_battle_with_empty_ai() {
    use tantalus_tools::battle_run::run_scene_battle;

    let mut scene = scene();
    for monster in &mut scene.monster_types {
        monster.max_hp = 60;
        monster.stats.spirit = 10;
    }
    // Minimal valid event archive: EV, version 2, zero entries.
    let mut script = b"EV\x02\x00".to_vec();
    script.resize(128, 0);

    let report = run_scene_battle("SYNTHETIC", &scene, 0, &script, 42, false, false);
    assert_eq!(report.error, None);
    assert_eq!(
        report.outcome,
        Some(tantalus_game::battle::BattleOutcome::Victory)
    );
    assert_eq!(report.enemy_deaths, 2);
    assert_eq!(report.bonus_gil, 300, "scene-declared spoils");
    assert_eq!(report.bonus_exp, 300);
    assert!(report.ticks < 20_000);
}

#[test]
fn choose_pattern_ports_the_reference_roll_and_group_override() {
    use tantalus_tools::battle_scene_rewards::choose_pattern;

    let mut scene = scene();
    let base = scene.patterns[0].clone();
    scene.patterns = vec![
        Pattern {
            rate: 60,
            ap: 1,
            ..base.clone()
        },
        Pattern {
            rate: 30,
            ap: 2,
            ..base.clone()
        },
        Pattern {
            rate: 10,
            ap: 3,
            ..base
        },
    ];
    // Cumulative walk: 60/30/10.
    assert_eq!(choose_pattern(&scene, None, 0), 0);
    assert_eq!(choose_pattern(&scene, None, 59), 0);
    assert_eq!(choose_pattern(&scene, None, 60), 1);
    assert_eq!(choose_pattern(&scene, None, 89), 1);
    assert_eq!(choose_pattern(&scene, None, 90), 2);
    assert_eq!(choose_pattern(&scene, None, 99), 2);
    // ENCOUNT2 group override, including the reference's clamps.
    assert_eq!(choose_pattern(&scene, Some(2), 0), 2);
    assert_eq!(choose_pattern(&scene, Some(9), 0), 0, ">= count -> 0");
    assert_eq!(choose_pattern(&scene, Some(-1), 99), 0, "negative -> 0");
    // A short-rate table returns the last pattern instead of reading out of
    // bounds (vanilla sums to 100; this is the guard).
    let mut short = scene.clone();
    short.patterns[2].rate = 0;
    assert_eq!(choose_pattern(&short, None, 99), 2);
}

#[test]
fn run_world_battle_plays_a_scene_through_the_field_world() {
    use tantalus_contracts::EntityId;
    use tantalus_game::FieldWorld;
    use tantalus_game::battle::legacy::PlayerBase;
    use tantalus_legacy::battle::WeaponStats;
    use tantalus_tools::battle_run::run_world_battle;

    let mut scene = scene();
    for monster in &mut scene.monster_types {
        monster.max_hp = 60;
        monster.stats.spirit = 10;
    }
    let mut script = b"EV\x02\x00".to_vec();
    script.resize(128, 0);

    let mut world = FieldWorld::new();
    world.party = vec![0, 1];
    world.gil = 10;
    for character in [0, 1] {
        world.character_stats.insert(
            character,
            tantalus_game::field::CharacterStats {
                current_hp: 500,
                max_hp: 500,
                current_mp: 100,
                max_mp: 100,
            },
        );
    }
    let entities = vec![EntityId::new(201), EntityId::new(202)];
    let base = |_| PlayerBase {
        level: 20,
        strength: 30,
        magic: 30,
        dexterity: 30,
        will: 20,
        physical_defence: 10,
        physical_evade: 5,
        magic_defence: 12,
        magic_evade: 6,
        weapon: Some(WeaponStats {
            power: 25,
            rate: 0,
            elements: tantalus_legacy::battle::Elements::NONE,
            category: WeaponStats::SHORT_RANGE,
            status: 0,
        }),
        learnable: Vec::new(),
        equipped_abilities: Vec::new(),
    };

    let (outcome, _ticks) =
        run_world_battle(&mut world, &entities, &scene, 0, &script, &base, 7, false)
            .expect("world battle runs clean");
    assert_eq!(outcome, Some(tantalus_game::battle::BattleOutcome::Victory));
    assert_eq!(world.gil, 10 + 300, "scene spoils committed to the world");
    // 300 EXP splits over the two participating party lines (the
    // participation mask follows the actual party, not a fixed four).
    assert_eq!(world.character_exp.get(&0).copied(), Some(150));
    assert_eq!(world.character_exp.get(&1).copied(), Some(150));
    // Surviving HP/MP carried back to the field world (the unhurt party
    // keeps its full current values, clamped to max).
    let stats = &world.character_stats[&0];
    assert_eq!((stats.current_hp, stats.current_mp), (500, 100));
}

#[test]
fn after_event_battles_pay_nothing() {
    use tantalus_contracts::EntityId;
    use tantalus_game::FieldWorld;
    use tantalus_game::battle::legacy::PlayerBase;
    use tantalus_legacy::battle::WeaponStats;
    use tantalus_tools::battle_run::run_world_battle;

    let mut scene = scene();
    for monster in &mut scene.monster_types {
        monster.max_hp = 60;
        monster.stats.spirit = 10;
    }
    scene.header.flags |= 0x800; // SB2_HEAD after_event
    let mut script = b"EV\x02\x00".to_vec();
    script.resize(128, 0);

    let mut world = FieldWorld::new();
    world.party = vec![0];
    world.gil = 500;
    world.character_stats.insert(
        0,
        tantalus_game::field::CharacterStats {
            current_hp: 500,
            max_hp: 500,
            current_mp: 100,
            max_mp: 100,
        },
    );
    let entities = vec![EntityId::new(201)];
    let base = |_| PlayerBase {
        level: 20,
        strength: 30,
        magic: 30,
        dexterity: 30,
        will: 20,
        weapon: Some(WeaponStats {
            power: 25,
            rate: 0,
            elements: tantalus_legacy::battle::Elements::NONE,
            category: WeaponStats::SHORT_RANGE,
            status: 0,
        }),
        ..PlayerBase::default()
    };
    let (outcome, _ticks) =
        run_world_battle(&mut world, &entities, &scene, 0, &script, &base, 3, false)
            .expect("world battle runs clean");
    assert_eq!(outcome, Some(tantalus_game::battle::BattleOutcome::Victory));
    assert_eq!(world.gil, 500, "after-event battles pay nothing");
    assert!(world.character_exp.is_empty(), "no EXP either");
}

#[test]
fn run_world_battle_applies_ap_to_equipped_learnables() {
    use tantalus_contracts::EntityId;
    use tantalus_game::FieldWorld;
    use tantalus_game::battle::legacy::PlayerBase;
    use tantalus_legacy::battle::WeaponStats;
    use tantalus_tools::battle_run::run_world_battle;

    let mut scene = scene();
    for monster in &mut scene.monster_types {
        monster.max_hp = 60;
        monster.stats.spirit = 10;
    }
    let mut script = b"EV\x02\x00".to_vec();
    script.resize(128, 0);

    let mut world = FieldWorld::new();
    world.party = vec![0];
    world.gil = 0;
    world.character_stats.insert(
        0,
        tantalus_game::field::CharacterStats {
            current_hp: 500,
            max_hp: 500,
            current_mp: 100,
            max_mp: 100,
        },
    );
    let entities = vec![EntityId::new(201)];
    let base = |_| PlayerBase {
        level: 20,
        strength: 30,
        magic: 30,
        dexterity: 30,
        will: 20,
        physical_defence: 10,
        physical_evade: 5,
        magic_defence: 12,
        magic_evade: 6,
        weapon: Some(WeaponStats {
            power: 25,
            rate: 0,
            elements: tantalus_legacy::battle::Elements::NONE,
            category: WeaponStats::SHORT_RANGE,
            status: 0,
        }),
        // Pattern AP is 3; ability 200 needs 2 -> learned by this battle.
        learnable: vec![(200, 2)],
        equipped_abilities: vec![200],
    };
    let (outcome, _ticks) =
        run_world_battle(&mut world, &entities, &scene, 0, &script, &base, 11, false)
            .expect("world battle runs clean");
    assert_eq!(outcome, Some(tantalus_game::battle::BattleOutcome::Victory));
    assert_eq!(world.ability_pa[&0][&200], 2, "AP reached max");
    assert!(
        world.learned_abilities[&0].contains(&200),
        "ability learned through the battle"
    );
}

#[test]
fn run_world_battle_applies_the_field_min_hp_rule() {
    use tantalus_contracts::EntityId;
    use tantalus_game::FieldWorld;
    use tantalus_game::battle::legacy::PlayerBase;
    use tantalus_legacy::battle::WeaponStats;
    use tantalus_tools::battle_run::run_world_battle;

    let mut scene = scene();
    for monster in &mut scene.monster_types {
        monster.max_hp = 60;
        monster.stats.spirit = 10;
    }
    let mut script = b"EV\x02\x00".to_vec();
    script.resize(128, 0);

    let mut world = FieldWorld::new();
    world.party = vec![0];
    world.gil = 500;
    // A 0-HP character is healed to 1 by the field rule before battle
    // (matching the chain engine's `party_unit` clamp): the party fights
    // and wins instead of entering dead.
    world.character_stats.insert(
        0,
        tantalus_game::field::CharacterStats {
            current_hp: 0,
            max_hp: 500,
            current_mp: 0,
            max_mp: 100,
        },
    );
    let entities = vec![EntityId::new(201)];
    let base = |_| PlayerBase {
        level: 20,
        strength: 30,
        magic: 30,
        dexterity: 30,
        will: 20,
        physical_defence: 10,
        physical_evade: 5,
        magic_defence: 12,
        magic_evade: 6,
        weapon: Some(WeaponStats {
            power: 25,
            rate: 0,
            elements: tantalus_legacy::battle::Elements::NONE,
            category: WeaponStats::SHORT_RANGE,
            status: 0,
        }),
        learnable: Vec::new(),
        equipped_abilities: Vec::new(),
    };
    let (outcome, _ticks) =
        run_world_battle(&mut world, &entities, &scene, 0, &script, &base, 5, false)
            .expect("world battle runs clean");
    assert_eq!(
        outcome,
        Some(tantalus_game::battle::BattleOutcome::Victory),
        "the field rule heals the party to 1 HP before battle"
    );
    assert_eq!(world.gil, 800, "the healed party earns the spoils");
    let stats = &world.character_stats[&0];
    assert_eq!(
        (stats.current_hp, stats.current_mp),
        (1, 0),
        "post-battle state reflects the 1-HP floor (no enemy attacked)"
    );
}

#[test]
fn sweep_compare_classifies_outcome_spoils_and_error_changes() {
    use tantalus_tools::sweep_compare::compare;

    let row = |outcome: &str, gil: i64, exp: i64, error: &str| {
        serde_json::json!({
            "scene": "S", "outcome": outcome,
            "bonus_gil": gil, "bonus_exp": exp, "error": error,
        })
    };
    let flat = |rows: &[serde_json::Value]| serde_json::json!({"results": rows});
    let patterned = |rows: &[serde_json::Value]| serde_json::json!({"results": [{"scene": "S", "patterns": rows}]});

    // Identical reports: no deltas, both shapes.
    let a = flat(&[row("victory", 100, 50, "")]);
    assert_eq!(compare(&a, &a)["identical"], true);

    // Outcome + spoils + error changes classified.
    let before = flat(&[row("victory", 100, 50, "")]);
    let after = flat(&[row("defeat", 0, 0, "tick 7: boom")]);
    let report = compare(&before, &after);
    assert_eq!(report["outcome_changes"], 1);
    assert_eq!(report["spoils_changes"], 1);
    assert_eq!(report["error_changes"], 1);
    assert_eq!(report["deltas"][0]["scene"], "S");

    // Per-pattern shapes compare pattern by pattern.
    let before = patterned(&[row("victory", 10, 5, ""), row("victory", 20, 5, "")]);
    let after = patterned(&[row("victory", 10, 5, ""), row("escape", 0, 0, "")]);
    let report = compare(&before, &after);
    assert_eq!(report["outcome_changes"], 1);
    assert_eq!(report["deltas"][0]["pattern"], 1);

    // Shape mismatch is explicit, not a crash.
    let report = compare(
        &flat(&[row("victory", 0, 0, "")]),
        &patterned(&[row("victory", 0, 0, ""), row("victory", 0, 0, "")]),
    );
    assert_eq!(report["same_shape"], false);
}

#[test]
fn world_battle_distinguishes_early_failure_from_no_outcome() {
    use tantalus_tools::battle_run::run_world_battle_report;

    let scene = scene();
    // Undecodable script: a genuine early failure, reported as an error.
    let report = run_world_battle_report(
        "EVT_BATTLE_TEST",
        &scene,
        0,
        b"garbage!",
        0x5EED,
        false,
        false,
    );
    assert!(
        report
            .error
            .as_deref()
            .is_some_and(|e| e.starts_with("script decode failed")),
        "early failure is an error, got {:?}",
        report.error
    );
    assert_eq!(report.outcome, None);
}

#[test]
fn back_attack_flag_zeroes_the_partys_initial_atb() {
    use tantalus_tools::battle_run::run_scene_battle;

    // `SB2_FLG_BACK_ATTACK` (0x2) forces StartType BackAttack
    // (btl_sys.cs:181-185), and BackAttack zeroes the players' initial ATB
    // (btl_init.cs:289-291) -- the party acts strictly later than in a
    // normal start, so the battle runs longer. Locks the start-type
    // wiring into the regression suite.
    let mut normal = scene();
    for monster in &mut normal.monster_types {
        monster.max_hp = 60;
        monster.stats.spirit = 10;
    }
    let mut back = normal.clone();
    back.header.flags |= 0x2;
    let mut script = b"EV ".to_vec();
    script.resize(128, 0);

    for seed in [0x5EEDu64, 7] {
        let a = run_scene_battle("EVT_BATTLE_TEST", &normal, 0, &script, seed, false, false);
        let b = run_scene_battle("EVT_BATTLE_TEST", &back, 0, &script, seed, false, false);
        assert_eq!(a.error, None, "seed {seed}: normal clean");
        assert_eq!(b.error, None, "seed {seed}: back-attack clean");
        assert_eq!(
            a.outcome,
            Some(tantalus_game::battle::BattleOutcome::Victory)
        );
        assert_eq!(
            b.outcome,
            Some(tantalus_game::battle::BattleOutcome::Victory)
        );
        assert!(
            b.ticks > a.ticks,
            "seed {seed}: back attack (b: {} ticks) must run longer than normal (a: {} ticks)",
            b.ticks,
            a.ticks
        );
    }
}

#[test]
fn no_run_away_flag_blocks_the_flee_roll() {
    use tantalus_tools::battle_run::{run_scene_battle, run_world_battle_report};

    // `SB2_FLG_NO_RUN_AWAY` (0x20): `Info.Runaway = (flags & 32) == 0`
    // (BTL_SCENE.cs:322). With the flag set the flee roll must not happen:
    // a flee-first party (which never attacks) can stall to the tick
    // budget, but it must NEVER escape. Regression lock for the runaway
    // fix (bosses were escapable before it).
    let mut scene = scene();
    scene.header.flags = 0x20;
    for monster in &mut scene.monster_types {
        monster.max_hp = 60;
        monster.stats.spirit = 10;
    }
    let mut script = b"EV ".to_vec();
    script.resize(128, 0);

    for seed in [0x5EEDu64, 9] {
        let report = run_scene_battle("EVT_BATTLE_TEST", &scene, 0, &script, seed, false, true);
        let world =
            run_world_battle_report("EVT_BATTLE_TEST", &scene, 0, &script, seed, false, true);
        assert_eq!(report.error, None, "seed {seed}: report path clean");
        assert_eq!(world.error, None, "seed {seed}: world path clean");
        assert_ne!(
            report.outcome,
            Some(tantalus_game::battle::BattleOutcome::Escape),
            "seed {seed}: no_run_away must block the report path's escape"
        );
        assert_ne!(
            world.outcome,
            Some(tantalus_game::battle::BattleOutcome::Escape),
            "seed {seed}: no_run_away must block the world path's escape"
        );
    }
}

#[test]
fn flee_first_paths_agree_on_the_escape_outcome() {
    use tantalus_tools::battle_run::{run_scene_battle, run_world_battle_report};

    // Weak party vs. an enemy it cannot out-damage: with flee-first the only
    // non-defeat exit is the flee key, so both paths must reach the same
    // outcome (Escape on a successful roll, Defeat otherwise) with no error
    // and no spoils either way.
    let mut scene = scene();
    for monster in &mut scene.monster_types {
        monster.max_hp = 6000;
        monster.stats.strength = 40;
    }
    let mut script = b"EV\x02\x00".to_vec();
    script.resize(128, 0);

    for seed in [0x5EEDu64, 9, 77] {
        let report = run_scene_battle("EVT_BATTLE_TEST", &scene, 0, &script, seed, true, true);
        let world =
            run_world_battle_report("EVT_BATTLE_TEST", &scene, 0, &script, seed, true, true);
        assert_eq!(report.error, None, "seed {seed}: report path clean");
        assert_eq!(world.error, None, "seed {seed}: world path clean");
        assert_eq!(report.outcome, world.outcome, "seed {seed}: outcome");
        assert_eq!(report.bonus_gil, world.bonus_gil, "seed {seed}: gil");
        assert_eq!(report.bonus_exp, world.bonus_exp, "seed {seed}: exp");
    }
}

#[test]
fn report_paths_agree_on_outcome_gil_and_exp() {
    use tantalus_tools::battle_run::{run_scene_battle, run_world_battle_report};

    let mut scene = scene();
    for monster in &mut scene.monster_types {
        monster.max_hp = 60;
        monster.stats.spirit = 10;
    }
    let mut script = b"EV\x02\x00".to_vec();
    script.resize(128, 0);

    // Same scene, same seeds, both engine paths: the report-only runner and
    // the world hand-off must agree on outcome, gil, and EXP. Item/card
    // draws are independent streams and stay un-compared.
    for seed in [0x5EEDu64, 1, 7] {
        let report = run_scene_battle("EVT_BATTLE_TEST", &scene, 0, &script, seed, false, false);
        let world =
            run_world_battle_report("EVT_BATTLE_TEST", &scene, 0, &script, seed, false, false);
        assert_eq!(report.error, None, "seed {seed}: report path clean");
        assert_eq!(world.error, None, "seed {seed}: world path clean");
        assert_eq!(report.outcome, world.outcome, "seed {seed}: outcome");
        assert_eq!(report.bonus_gil, world.bonus_gil, "seed {seed}: gil");
        assert_eq!(report.bonus_exp, world.bonus_exp, "seed {seed}: exp");
    }
}

#[test]
fn chain_and_handoff_paths_agree_on_the_same_scene() {
    use std::collections::BTreeMap;
    use tantalus_assets::pc::battle_scene::BattleBundleData;
    use tantalus_contracts::EntityId;
    use tantalus_game::FieldWorld;
    use tantalus_legacy::battle::WeaponStats;
    use tantalus_tools::battle_run::run_world_battle;
    use tantalus_tools::field_battle::{Equipment, FieldBattles, PartyFighter, save_party};

    let mut scene = scene();
    for monster in &mut scene.monster_types {
        monster.max_hp = 60;
        monster.stats.spirit = 10;
    }
    let mut script = b"EV\x02\x00".to_vec();
    script.resize(128, 0);

    let base_stats = tantalus_game::new_game::BaseStats {
        speed: 23,
        strength: 21,
        magic: 18,
        spirit: 23,
        gems: 18,
    };

    // The chain engine derives its RNG streams from the battle id; the
    // hand-off runner takes its seed explicitly. Feeding the same numeric
    // value through both paths exercises several independent RNG streams
    // per path without touching Opus's seed plumbing.
    // Item/card drops are intentionally NOT compared: each path owns an
    // independent draw stream, so drop tables may differ by design.
    for seed in 0..5u16 {
        // Opus's chain path: FieldBattles::fight.
        let engine = FieldBattles::from_parts(
            BattleBundleData {
                scenes: BTreeMap::from([("EVT_BATTLE_TEST".to_owned(), Ok(scene.clone()))]),
                sequences: BTreeMap::new(),
                sequence_bytes: BTreeMap::new(),
                infos: BTreeMap::new(),
            },
            BTreeMap::from([("EVT_BATTLE_TEST".to_owned(), script.clone())]),
            BTreeMap::from([(seed, "BSC_TEST".to_owned())]),
            Equipment::default(),
        );
        let report = engine.fight(
            &tantalus_game::field::world::EncounterRequest {
                rush: 0,
                group: None,
                battle: i32::from(seed),
                steiner_state: false,
                timer: None,
                pattern_roll: 0,
            },
            &[PartyFighter {
                character: 0,
                hp: 500,
                max_hp: 500,
                mp: 100,
                max_mp: 100,
                base: base_stats,
                level: 1,
                globals: Default::default(),
                row: 1,
                trance: 0,
                equip: None,
                support: 0,
            }],
        );
        assert_eq!(report.error, None, "seed {seed}: chain path ran clean");
        assert_eq!(
            report.outcome.as_deref(),
            Some("victory"),
            "seed {seed}: chain path victory"
        );

        // My hand-off path: run_world_battle over the same scene.
        let mut world = FieldWorld::new();
        world.party = vec![0];
        world.gil = 0;
        world.character_stats.insert(
            0,
            tantalus_game::field::CharacterStats {
                current_hp: 500,
                max_hp: 500,
                current_mp: 100,
                max_mp: 100,
            },
        );
        let (outcome, _ticks) = run_world_battle(
            &mut world,
            &[EntityId::new(201)],
            &scene,
            0,
            &script,
            &|_| tantalus_game::battle::legacy::PlayerBase {
                level: 20,
                strength: 30,
                magic: 30,
                dexterity: 30,
                will: 20,
                physical_defence: 10,
                physical_evade: 5,
                magic_defence: 12,
                magic_evade: 6,
                weapon: Some(WeaponStats {
                    power: 25,
                    rate: 0,
                    elements: tantalus_legacy::battle::Elements::NONE,
                    category: WeaponStats::SHORT_RANGE,
                    status: 0,
                }),
                learnable: Vec::new(),
                equipped_abilities: Vec::new(),
            },
            u64::from(seed),
            false,
        )
        .expect("world battle runs clean");
        assert_eq!(
            outcome,
            Some(tantalus_game::battle::BattleOutcome::Victory),
            "seed {seed}: both paths agree on the outcome"
        );

        // Gil and EXP are draw-independent: both paths must agree exactly.
        assert_eq!(
            report.reward_gil, world.gil,
            "seed {seed}: both paths pay the same gil ({} vs {})",
            report.reward_gil, world.gil
        );
        let chain_exp: u32 = report.reward_exp.iter().map(|(_, exp)| *exp).sum();
        let handoff_exp: u64 = world.character_exp.values().copied().sum::<u64>() as u32 as u64;
        assert_eq!(
            u64::from(chain_exp),
            handoff_exp,
            "seed {seed}: both paths split EXP identically"
        );

        // Survivor HP/MP carry-back: Opus's `save_party` (report ->
        // clamped record write) must land on the same numbers the
        // hand-off runner writes in place.
        let chain_stats = BTreeMap::from([(
            0,
            tantalus_game::field::CharacterStats {
                current_hp: 500,
                max_hp: 500,
                current_mp: 100,
                max_mp: 100,
            },
        )]);
        let mut chain_world = tantalus_game::FieldWorld::new();
        chain_world.character_stats = chain_stats;
        save_party(&report, &mut chain_world);
        let chain = &chain_world.character_stats[&0];
        let handoff = &world.character_stats[&0];
        assert_eq!(
            (chain.current_hp, chain.current_mp),
            (handoff.current_hp, handoff.current_mp),
            "seed {seed}: survivor HP/MP carry-back agrees"
        );
    }
}
