use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use tantalus_contracts::*;
use tantalus_core::*;

fn id(text: &str) -> ContentId {
    ContentId::parse(text).unwrap()
}

fn profile() -> ProfileLock {
    ProfileLock {
        content_revision: ContentRevision::from_u64(4),
        content_fingerprint: "fixture-content-digest".to_owned(),
        modules: vec![
            ModuleLock {
                package: id("fixture:story/required"),
                version: "1.0.0".to_owned(),
                required: true,
                state_schema: 3,
            },
            ModuleLock {
                package: id("fixture:cosmetic/optional"),
                version: "2.0.0".to_owned(),
                required: false,
                state_schema: 1,
            },
        ],
    }
}

fn snapshot() -> SaveEnvelope {
    let mut first_properties = InstanceProperties::default();
    first_properties
        .stats
        .insert(id("fixture:stat/power"), PropertyValue::Integer(7));
    first_properties.slot = Some(id("fixture:slot/rellic"));
    let mut second_properties = InstanceProperties::default();
    second_properties
        .stats
        .insert(id("fixture:stat/power"), PropertyValue::Integer(11));

    let mut progression = BTreeMap::new();
    progression.insert(id("fixture:progress/gil"), PropertyValue::Integer(0));
    let mut modules = BTreeMap::new();
    modules.insert(
        id("fixture:story/required"),
        ModuleSaveSection {
            schema_version: 3,
            state: json!({"quest_stage": 2}),
        },
    );
    modules.insert(
        id("fixture:cosmetic/optional"),
        ModuleSaveSection {
            schema_version: 1,
            state: json!({"palette": "amber"}),
        },
    );

    SaveEnvelope {
        format_version: CURRENT_SAVE_FORMAT_VERSION,
        engine: EngineIdentity {
            name: "tantalus".to_owned(),
            version: "0.1.0".to_owned(),
        },
        ruleset: id("fixture:rules/default"),
        profile: profile(),
        checkpoint: CheckpointMetadata {
            kind: CheckpointKind::SuspendedSession,
            tick: Tick::new(120),
            resume_providers: vec![id("fixture:provider/encounter")],
        },
        state: AuthoritativeState {
            entities: vec![EntitySaveState {
                id: EntityId::new(1),
                definition: id("fixture:actor/raptor"),
                active: false,
                position: Point3 {
                    x: 20,
                    y: 0,
                    z: -40,
                },
                properties: BTreeMap::new(),
            }],
            items: vec![
                ItemInstanceSaveState {
                    id: ItemInstanceId::from_u64(1).unwrap(),
                    definition: id("fixture:weapon/same_definition"),
                    owner: Some(EntityId::new(1)),
                    properties: first_properties,
                },
                ItemInstanceSaveState {
                    id: ItemInstanceId::from_u64(2).unwrap(),
                    definition: id("fixture:weapon/same_definition"),
                    owner: None,
                    properties: second_properties,
                },
            ],
            encounters: vec![EncounterSaveState {
                id: EncounterId::new(8),
                provider: id("fixture:provider/encounter"),
                state: json!({"phase": "returning", "defeated": [1]}),
            }],
            rng: RngSaveState {
                algorithm: "fixture-prng".to_owned(),
                state: vec![123, 456],
                position: 9,
            },
            progression,
            world_flags: BTreeMap::new(),
        },
        modules,
        migration_history: Vec::new(),
        unknown_sections: vec![UnknownSaveSection {
            name: "future.optional.telemetry".to_owned(),
            schema_version: 99,
            state: json!({"retain": [1, 2, 3]}),
        }],
        outcomes: Vec::new(),
    }
}

fn rewarded(mut snapshot: SaveEnvelope) -> SaveEnvelope {
    snapshot
        .state
        .progression
        .insert(id("fixture:progress/gil"), PropertyValue::Integer(100));
    snapshot.outcomes.push(CommittedOutcome {
        resolution_id: 42,
        encounter: EncounterId::new(8),
        provider: id("fixture:provider/encounter"),
        result: json!({"gil": 100}),
    });
    snapshot
}

fn load_ready(store: &SaveStore, profile: &ProfileLock) -> SaveEnvelope {
    store
        .load(store.latest().unwrap().unwrap(), profile)
        .unwrap()
        .into_ready()
        .unwrap()
}

#[test]
fn non_ascii_round_trip_preserves_instances_modules_and_unknown_data() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = SaveStore::new(temp.path().join("saves-日本語"));
    let original = snapshot();
    let save_id = store.commit(&original).unwrap();
    let loaded = store
        .load(save_id, &profile())
        .unwrap()
        .into_ready()
        .unwrap();

    assert_eq!(loaded, original);
    assert_eq!(loaded.state.items.len(), 2);
    assert_eq!(
        loaded.state.items[0].definition,
        loaded.state.items[1].definition
    );
    assert_ne!(
        loaded.state.items[0].properties,
        loaded.state.items[1].properties
    );
    assert_eq!(
        loaded.unknown_sections[0].state,
        json!({"retain": [1, 2, 3]})
    );
}

#[test]
fn outcome_recovery_is_exactly_once_at_every_publication_boundary() {
    for point in [
        SaveFailurePoint::BeforeGenerationPublish,
        SaveFailurePoint::BeforeLatestPublish,
        SaveFailurePoint::AfterLatestPublish,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut store = SaveStore::new(temp.path());
        store.commit(&snapshot()).unwrap();
        store.fail_next_commit(point);
        assert!(store.commit_outcome(&rewarded(snapshot()), 42).is_err());

        let after_failure = load_ready(&store, &profile());
        if point == SaveFailurePoint::AfterLatestPublish {
            assert_eq!(after_failure.outcomes.len(), 1);
            assert_eq!(
                after_failure.state.progression[&id("fixture:progress/gil")],
                PropertyValue::Integer(100)
            );
            let duplicate = store.commit_outcome(&rewarded(snapshot()), 42).unwrap_err();
            assert_eq!(duplicate.category, SaveErrorCategory::Conflict);
        } else {
            assert!(after_failure.outcomes.is_empty());
            assert_eq!(
                after_failure.state.progression[&id("fixture:progress/gil")],
                PropertyValue::Integer(0)
            );
            store.commit_outcome(&rewarded(snapshot()), 42).unwrap();
        }

        let recovered = load_ready(&store, &profile());
        assert_eq!(recovered.outcomes.len(), 1);
        assert_eq!(recovered.outcomes[0].resolution_id, 42);
        assert_eq!(
            recovered.state.progression[&id("fixture:progress/gil")],
            PropertyValue::Integer(100)
        );
    }
}

#[test]
fn corrupt_latest_generation_falls_back_without_rewriting_history() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = SaveStore::new(temp.path());
    let first = store.commit(&snapshot()).unwrap();
    let mut second = snapshot();
    second.state.rng.position = 10;
    let second = store.commit(&second).unwrap();
    let latest_path = temp
        .path()
        .join("generations")
        .join(format!("{:020}.json", second.get()));
    fs::write(&latest_path, b"truncated").unwrap();

    let recovered = store
        .recover_latest(&profile())
        .unwrap()
        .into_ready()
        .unwrap();
    assert_eq!(recovered, snapshot());
    assert_eq!(fs::read(&latest_path).unwrap(), b"truncated");
    assert_eq!(store.load(first, &profile()).unwrap().save_id, first);
}

#[test]
fn missing_required_modules_block_resume_and_preserve_their_sections() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = SaveStore::new(temp.path());
    store.commit(&snapshot()).unwrap();
    let mut reduced_profile = profile();
    reduced_profile.modules.retain(|module| !module.required);

    let plan = store
        .load(store.latest().unwrap().unwrap(), &reduced_profile)
        .unwrap();
    assert_eq!(
        plan.missing_required_modules,
        vec![id("fixture:story/required")]
    );
    assert_eq!(
        plan.envelope.modules[&id("fixture:story/required")].state,
        json!({"quest_stage": 2})
    );
    let error = plan.into_ready().unwrap_err();
    assert_eq!(error.category, SaveErrorCategory::MissingRequiredModule);
    assert!(
        load_ready(&store, &profile())
            .modules
            .contains_key(&id("fixture:story/required"))
    );
}

fn downgrade_generation(root: &std::path::Path, id: SaveId) {
    let path = root
        .join("generations")
        .join(format!("{:020}.json", id.get()));
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["envelope"]["format_version"] = json!(0);
    let envelope: SaveEnvelope = serde_json::from_value(record["envelope"].clone()).unwrap();
    let bytes = serde_json::to_vec(&envelope).unwrap();
    record["digest"] = json!(format!("{:x}", Sha256::digest(bytes)));
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
}

#[test]
fn failed_migration_preserves_source_and_success_is_recorded() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = SaveStore::new(temp.path());
    let save_id = store.commit(&snapshot()).unwrap();
    downgrade_generation(temp.path(), save_id);
    let path = temp
        .path()
        .join("generations")
        .join(format!("{:020}.json", save_id.get()));
    let before = fs::read(&path).unwrap();

    let plan = store.load(save_id, &profile()).unwrap();
    assert!(plan.migration_required);
    assert!(plan.clone().into_ready().is_err());
    assert!(plan.clone().migrate(&MigrationRegistry::default()).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);

    let mut migrations = MigrationRegistry::default();
    migrations
        .register(0, |mut envelope| {
            envelope.format_version = 1;
            Ok(envelope)
        })
        .unwrap();
    let migrated = plan.migrate(&migrations).unwrap();
    assert_eq!(migrated.format_version, CURRENT_SAVE_FORMAT_VERSION);
    assert_eq!(migrated.migration_history.len(), 1);
    assert_eq!(migrated.migration_history[0].from_version, 0);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn io_failure_does_not_replace_an_authoritative_save() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = SaveStore::new(temp.path());
    store.commit(&snapshot()).unwrap();
    let pointer_before = fs::read(temp.path().join("current.json")).unwrap();
    fs::remove_dir_all(temp.path().join("generations")).unwrap();
    fs::write(temp.path().join("generations"), b"not a directory").unwrap();

    assert!(store.commit(&snapshot()).is_err());
    assert_eq!(
        fs::read(temp.path().join("current.json")).unwrap(),
        pointer_before
    );
}

#[test]
fn duplicate_identity_is_rejected_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = SaveStore::new(temp.path());
    let mut invalid = snapshot();
    invalid.state.items.push(invalid.state.items[0].clone());
    assert!(store.commit(&invalid).is_err());
    assert_eq!(store.latest().unwrap(), None);
    assert!(!temp.path().join("current.json").exists());
}
