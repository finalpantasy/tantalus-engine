use super::*;
use crate::pc::battle_scene::{ByteOrTokens, MonsterStats, MonsterType, Pattern, SceneHeader};
use crate::pc::battle_sequence::SequenceFile;

fn clip(geo_id: u32, name: &str) -> ClipEntry {
    ClipEntry {
        geo_id,
        name: name.to_owned(),
        path_id: 1,
    }
}

fn entry(path: &str, class_id: u16) -> ContainerEntry {
    ContainerEntry {
        path: path.to_owned(),
        path_id: 1,
        class_id,
    }
}

fn monster(geo: i16, mot: [u16; 6]) -> MonsterType {
    MonsterType {
        resist_status: 0,
        auto_status: 0,
        initial_status: 0,
        max_hp: 1,
        max_mp: 0,
        win_gil: 0,
        win_exp: 0,
        win_items: [0; 4],
        steal_items: [0; 4],
        win_item_rates: [0; 4],
        steal_item_rates: [0; 4],
        win_card_rate: 0,
        radius: 0,
        geo,
        mot,
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
        level: 1,
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
        win_card: 0,
        shadow_ofs_x: 0,
        shadow_ofs_z: 0,
        shadow_bone2: 0,
        pad0: 0,
        pad1: 0,
        pad2: 0,
    }
}

fn placement(type_no: u8, flags: u8) -> Placement {
    Placement {
        type_no,
        flags,
        pease: 0,
        pad: 0,
        x: 0,
        y: 0,
        z: 0,
        rot: 0,
    }
}

fn scene(types: Vec<MonsterType>, placements: &[(u8, u8)]) -> BattleScene {
    let mut monsters = [
        placement(0, 0),
        placement(0, 0),
        placement(0, 0),
        placement(0, 0),
    ];
    for (slot, (type_no, flags)) in placements.iter().enumerate() {
        monsters[slot] = placement(*type_no, *flags);
    }
    BattleScene {
        header: SceneHeader {
            version: 7,
            pattern_count: 1,
            type_count: types.len() as u8,
            attack_count: 0,
            flags: 0,
            pad1: 0,
            battle_map_index: None,
        },
        patterns: vec![Pattern {
            rate: 1,
            monster_count: placements.len() as u8,
            camera: 0,
            pad0: 0,
            ap: 0,
            monsters,
        }],
        monster_types: types,
        attacks: Vec::new(),
        trailing_bytes: 0,
        trailing_all_zero: true,
        file_len: 0,
        trailing: Vec::new(),
    }
}

fn has_root(dir: &ModelDir) -> bool {
    !dir.roots.is_empty()
}

fn models_catalog() -> ModelCatalog {
    ModelCatalog::from_container(&[
        // Geo 233 is `GEO_ACC_F0_OPB`, an `acc` (type 1) directory.
        entry("assets/resources/models/1/233/233.fbx", 1),
        entry("assets/resources/models/1/233/233.fbx", 43),
    ])
}

#[test]
fn name_tables_follow_the_source_rows() {
    assert_eq!(battle_animation_name(125), Some("ANH_MAIN_B0_000_000"));
    assert_eq!(battle_animation_name(0), None);
    assert_eq!(battle_animation_count(), 4629);
    assert_eq!(animation_db_key("ANH_MAIN_B0_000_000"), Some(125));
    assert_eq!(geo_id_by_name("GEO_MAIN_B0_000"), Some(5414));
    assert_eq!(background_name("ac_e031"), Some("BBG_B009"));
    assert_eq!(background_name("NO_SUCH"), None);
    assert_eq!(background_table_len(), 562);
    assert_eq!(special_effect_name(200), Some("Steal_Zidane_Dagger"));
    assert_eq!(special_effect_id("Steal_Zidane_Dagger"), Some(200));
    assert_eq!(special_effect_name(-1), Some("Special_No_Effect"));
}

#[test]
fn animation_resolves_through_name_key_and_folder() {
    let clips = ClipIndex::new(&[clip(5414, "125"), clip(7, "not-a-number")]);
    assert_eq!(clips.len(), 1);
    let found = resolve_animation_id(125, Some(&clips));
    assert_eq!(found.status, AnimationStatus::Resolved);
    assert_eq!(found.folder_geo_id, Some(5414));
    assert_eq!(found.clip_key, Some(125));

    let wrong_folder = ClipIndex::new(&[clip(9, "125")]);
    let missing = resolve_animation_id(125, Some(&wrong_folder));
    assert_eq!(missing.status, AnimationStatus::ClipMissing);
    assert_eq!(missing.clip_found_in_other_folders, vec![9]);

    assert_eq!(
        resolve_animation_id(125, None).status,
        AnimationStatus::NotChecked
    );
    assert_eq!(
        resolve_animation_id(0, Some(&clips)).status,
        AnimationStatus::NotInBattleAnimationTable
    );
    assert_eq!(
        resolve_animation_name("ANH_SHORT", None, Some(&clips)).status,
        AnimationStatus::UnknownFolder
    );
    assert_eq!(
        resolve_animation_name("ANH_MAIN_B0_000_NOPE", None, Some(&clips)).status,
        AnimationStatus::NameNotInAnimationDb
    );
}

fn background_info(objanim: i16, texanim: i16) -> BattleInfo {
    BattleInfo {
        bbgnumber: 9,
        texanim,
        skyrotation: 0,
        fog: 0,
        objanim,
        uvcount: 0,
        chr_r: 0,
        chr_g: 0,
        chr_b: 0,
        shadow: 0,
        trailing_bytes: 0,
    }
}

#[test]
fn background_needs_model_info_objects_and_table() {
    let dir = "assets/resources/battlemap/battlemodel/battlemap_all/bbg_b009/";
    let mut catalog = BackgroundCatalog::from_container(&[
        entry(&format!("{dir}bbg_b009.fbx"), 1),
        entry(&format!("{dir}bbg_b009.fbx"), 43),
        entry(&format!("{dir}mat.mat"), 21),
        entry(&format!("{dir}image0.png"), 28),
        entry(
            "assets/resources/battlemap/battletexanim/tam_b009.tab.bytes",
            49,
        ),
    ]);
    assert_eq!(catalog.dirs["BBG_B009"].textures, 1);
    assert!(catalog.tex_anims.contains("TAM_B009"));

    // Scene AC_E031 maps to BBG_B009.
    let none = resolve_background("AC_E031", Some(&catalog));
    assert_eq!(none.status, BackgroundStatus::InfoMissing);

    catalog
        .infos
        .insert("INB_B009".into(), Ok(background_info(0, 1)));
    let ok = resolve_background("AC_E031", Some(&catalog));
    assert_eq!(ok.status, BackgroundStatus::Resolved);
    assert!(!ok.info_number_differs);

    catalog
        .infos
        .insert("INB_B009".into(), Ok(background_info(1, 1)));
    assert_eq!(
        resolve_background("AC_E031", Some(&catalog)).status,
        BackgroundStatus::ObjectModelMissing
    );

    catalog.tex_anims.clear();
    catalog
        .infos
        .insert("INB_B009".into(), Ok(background_info(0, 2)));
    assert_eq!(
        resolve_background("AC_E031", Some(&catalog)).status,
        BackgroundStatus::TexAnimMissing
    );
    // texanim 0 does not need the table.
    catalog
        .infos
        .insert("INB_B009".into(), Ok(background_info(0, 0)));
    assert_eq!(
        resolve_background("AC_E031", Some(&catalog)).status,
        BackgroundStatus::Resolved
    );

    catalog.dirs.get_mut("BBG_B009").unwrap().unreadable = vec!["mesh 5".into()];
    assert_eq!(
        resolve_background("AC_E031", Some(&catalog)).status,
        BackgroundStatus::ModelMissing
    );
    // A texture in an unsupported format is present data: it stays Resolved and is counted.
    {
        let dir = catalog.dirs.get_mut("BBG_B009").unwrap();
        dir.unreadable.clear();
        dir.undecoded_textures = vec!["image12.png: unsupported format".into()];
    }
    let undecoded = resolve_background("AC_E031", Some(&catalog));
    assert_eq!(undecoded.status, BackgroundStatus::Resolved);
    assert_eq!(undecoded.undecoded_textures, 1);
    catalog
        .dirs
        .get_mut("BBG_B009")
        .unwrap()
        .undecoded_textures
        .clear();
    assert_eq!(
        resolve_background("NO_SUCH", Some(&catalog)).status,
        BackgroundStatus::NoMapModelRow
    );
    assert_eq!(
        resolve_background("AC_E031", None).status,
        BackgroundStatus::NotChecked
    );
}

#[test]
fn scene_report_counts_models_animations_and_slaves() {
    // Type 0 is placed with geo 233 (resolves), type 1 is a multipart slave (no model needed,
    // its geo would not resolve), type 2 is in no pattern.
    let types = vec![
        monster(233, [125, 125, 0, 125, 125, 125]),
        monster(999, [125; 6]),
        monster(-1, [125; 6]),
    ];
    let scene = scene(types, &[(0, 0), (1, Placement::FLG_MULTIPART)]);
    let catalog = models_catalog();
    let clips = ClipIndex::new(&[clip(5414, "125")]);
    let facts = Facts {
        catalog: Some(&catalog),
        assembled: &has_root,
        clips: Some(&clips),
        backgrounds: None,
    };
    let report = build_scene_report(
        &[SceneInput {
            name: "AC_E031",
            scene: Ok(&scene),
            sequence: None,
        }],
        &facts,
    );
    let t = &report.totals;
    assert_eq!(t.scenes_read, 1);
    assert_eq!(t.enemy_types, 3);
    assert_eq!(t.enemy_types_in_pattern, 2);
    assert_eq!(t.enemy_models.references, 1);
    assert_eq!(t.enemy_models.references_resolved, 1);
    assert_eq!(t.enemy_animations.references, 18);
    // Animation id 0 is not in the table: one missing reference in type 0.
    assert_eq!(t.enemy_animations.references_resolved, 17);
    assert_eq!(t.enemy_animations.distinct, 2);
    assert_eq!(t.enemy_animations.distinct_resolved, 1);
    assert_eq!(report.missing.len(), 1);
    let item = &report.missing[0];
    assert_eq!(
        (item.kind, item.type_no, item.id),
        ("enemy_animation", Some(0), Some(0))
    );
    assert_eq!(item.reason, "not_in_battle_animation_table");
    assert!(item.in_pattern);
    assert_eq!(report.scenes[0].unresolved, 1);
    assert_eq!(t.scenes_fully_resolved, 0);
    assert!(report.scenes[0].enemies[2].geo_id < 0 && !report.scenes[0].enemies[2].in_pattern);
}

#[test]
fn missing_model_and_negative_geo_are_listed_with_reasons() {
    let types = vec![monster(998, [125; 6]), monster(-3, [125; 6])];
    let scene = scene(types, &[(0, 0), (1, 0)]);
    let catalog = models_catalog();
    let facts = Facts {
        catalog: Some(&catalog),
        assembled: &has_root,
        clips: None,
        backgrounds: None,
    };
    let report = build_scene_report(
        &[SceneInput {
            name: "ZZ_TEST",
            scene: Ok(&scene),
            sequence: None,
        }],
        &facts,
    );
    let reasons: Vec<_> = report
        .missing
        .iter()
        .filter(|m| m.kind == "enemy_model")
        .map(|m| (m.type_no, m.reason.as_str()))
        .collect();
    assert!(reasons.contains(&(Some(1), "negative_geo_id")));
    assert_eq!(report.totals.enemy_models.references, 2);
    assert_eq!(report.totals.enemy_models.references_resolved, 0);
    // Scene ZZ_TEST has no MapModel row.
    assert!(
        report
            .missing
            .iter()
            .any(|m| m.kind == "background" && m.reason.starts_with("no_map_model_row"))
    );
}

#[test]
fn unreadable_scene_is_a_missing_item() {
    let facts = Facts {
        catalog: None,
        assembled: &|_| false,
        clips: None,
        backgrounds: None,
    };
    let report = build_scene_report(
        &[SceneInput {
            name: "AC_E031",
            scene: Err("bad header".into()),
            sequence: None,
        }],
        &facts,
    );
    assert_eq!(report.totals.scenes_read, 0);
    assert_eq!(report.missing[0].kind, "scene_unreadable");
    assert_eq!(report.missing[0].reason, "bad header");
}

/// Builds a `.raw17` with one sequence slot: `Anim { code }` operands, then End.
fn sequence_file(base_anim: u8, anim_list: &[i32], codes: &[u8]) -> SequenceFile {
    let mut stream = Vec::new();
    for code in codes {
        stream.extend_from_slice(&[5, *code]);
    }
    stream.push(0);
    let seq_count = 1_usize;
    let header_len = 8 + 2 * seq_count + 4 * anim_list.len() + seq_count;
    // The reader starts a stream at `offset + 4`.
    let start = header_len + 4;
    let offset = (start - 4) as i16;
    let cam = (start + stream.len()) as i16;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0_i16.to_le_bytes());
    bytes.extend_from_slice(&cam.to_le_bytes());
    bytes.extend_from_slice(&(seq_count as i16).to_le_bytes());
    bytes.extend_from_slice(&(anim_list.len() as i16).to_le_bytes());
    bytes.extend_from_slice(&offset.to_le_bytes());
    for id in anim_list {
        bytes.extend_from_slice(&id.to_le_bytes());
    }
    bytes.push(base_anim);
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&stream);
    bytes.extend_from_slice(&[0, 0]);
    SequenceFile::parse(&bytes, "test").expect("synthetic sequence parses")
}

#[test]
fn sequence_animations_use_base_offset_and_skip_idle_code() {
    // Slot base 1: code 0 -> animList[1] = 125, code 1 -> animList[2] = 3 (not in table),
    // code 255 loads nothing, code 9 is past the list.
    let file = sequence_file(1, &[7, 125, 3], &[0, 1, 255, 9]);
    let scene = scene(vec![monster(233, [125; 6])], &[(0, 0)]);
    let catalog = models_catalog();
    let clips = ClipIndex::new(&[clip(5414, "125")]);
    let facts = Facts {
        catalog: Some(&catalog),
        assembled: &has_root,
        clips: Some(&clips),
        backgrounds: None,
    };
    let report = build_scene_report(
        &[SceneInput {
            name: "AC_E031",
            scene: Ok(&scene),
            sequence: Some(Ok(&file)),
        }],
        &facts,
    );
    let coverage = &report.scenes[0];
    assert_eq!(
        coverage
            .sequence_animation_ids
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![3, 125]
    );
    assert_eq!(coverage.sequence_anim_index_out_of_range, 1);
    assert_eq!(report.totals.sequence_animations.references, 2);
    assert_eq!(report.totals.sequence_animations.references_resolved, 1);
    let kinds: Vec<_> = report.missing.iter().map(|m| m.kind).collect();
    assert!(kinds.contains(&"sequence_animation"));
    assert!(kinds.contains(&"sequence_anim_index"));
}

#[test]
fn players_cover_nineteen_rows_with_thirty_four_animations() {
    let report = build_player_report(None, &|_| false, None, &|_| false, None, &[]);
    assert_eq!(report.players.len(), 19);
    assert!(report.players.iter().all(|p| p.animations.len() == 34));
    assert_eq!(report.totals.model_references, 38);
    // Without catalogs every model has a geo name and is "not checked".
    assert_eq!(report.totals.model_references_resolved, 38);
    assert_eq!(report.players[0].model.name, "GEO_MAIN_B0_000");
    assert_eq!(report.players[0].attack_sequence, 100);
    assert_eq!(player_attack_sequences().len(), 19);
}

#[test]
fn players_report_missing_clips_and_models() {
    // `GEO_MAIN_B0_000` (5414) loads the field model directory 2/98 (`model_factory`).
    let catalog = ModelCatalog::from_container(&[entry("assets/resources/models/2/98/98.fbx", 1)]);
    let clips = ClipIndex::new(&[clip(5414, "125")]);
    let report = build_player_report(
        Some(&catalog),
        &has_root,
        None,
        &|_| false,
        Some(&clips),
        &[],
    );
    let zidane = &report.players[0];
    assert_eq!(zidane.model.status, ModelStatus::Resolved);
    // The trance model has no directory in this catalog.
    assert_eq!(zidane.trance_model.status, ModelStatus::NoDirectory);
    assert!(zidane.unresolved > 1);
    assert_eq!(zidane.animations[0].status, AnimationStatus::Resolved);
}

#[test]
fn weapons_distinguish_dummy_unknown_and_missing_directories() {
    let catalog = ModelCatalog::from_container_with_prefix(
        &[
            entry("assets/resources/battlemap/battlemodel/6/264/264.fbx", 1),
            entry(
                "assets/resources/battlemap/battlemodel/battlemap_all/x/y.fbx",
                1,
            ),
        ],
        WEAPON_MODEL_PREFIX,
    );
    assert_eq!(catalog.dirs.len(), 1);
    assert_eq!(catalog.dirs["6/264"].type_name, "battle_weapon");
    let weapon_name = names::GEO_FIRST_ID
        .iter()
        .find(|(_, id)| *id == 264)
        .map(|(name, _)| (*name).to_owned());
    let weapons = vec![
        WeaponInput {
            id: 0,
            model_name: weapon_name,
        },
        WeaponInput {
            id: 1,
            model_name: None,
        },
        WeaponInput {
            id: 2,
            model_name: Some("GEO_WEP_NOPE".into()),
        },
    ];
    let report = build_player_report(None, &|_| false, Some(&catalog), &has_root, None, &weapons);
    let statuses: Vec<_> = report.weapons.iter().map(|w| w.status).collect();
    assert_eq!(
        statuses,
        vec![
            WeaponStatus::Resolved,
            WeaponStatus::NoModelByDesign,
            WeaponStatus::NoGeoName
        ]
    );
    assert_eq!(report.totals.weapons_with_model, 2);
    assert_eq!(report.totals.weapons_resolved, 1);
}

#[test]
fn effect_report_matches_ids_to_assets() {
    let reference = |source: &str, effect_id: i32| EffectReference {
        source: source.to_owned(),
        effect_id,
    };
    let references = vec![
        reference("aa_data[1].info.vfx_no", 200),
        reference("aa_data[2].info.vfx_no", 200),
        reference("aa_data[3].info.vfx_no", 777),
        reference("aa_data[4].vfx2", -1),
    ];
    let assets = BTreeMap::from([(200, 1000_usize), (5, 10)]);
    let report = build_effect_report(&references, &assets);
    let t = &report.totals;
    assert_eq!((t.references, t.no_effect_references), (4, 1));
    assert_eq!((t.distinct_effects, t.distinct_effects_present), (2, 1));
    assert_eq!(t.effects_not_in_enum, 1);
    assert_eq!((t.assets_in_resources, t.assets_unreferenced), (2, 1));
    assert_eq!(report.missing.len(), 1);
    assert_eq!(report.effects[0].enum_name, Some("Steal_Zidane_Dagger"));
    assert_eq!(report.effects[0].referenced_by, 2);
    for name in FIXED_PLAYER_EFFECTS {
        assert!(special_effect_id(name).is_some(), "{name}");
    }
}

#[test]
fn party_models_used_as_enemies_are_known_by_geo_id() {
    // `FF9BattleDB.GEO`: 298 GEO_MON_B3_148, 639 GEO_MON_B3_195; 290 is Freya.
    assert!(is_use_as_enemy_character(298));
    assert!(is_use_as_enemy_character(639));
    assert!(!is_use_as_enemy_character(290));
    assert!(!is_use_as_enemy_character(-1));
}
