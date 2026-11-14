use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use tantalus_contracts::*;
use tantalus_mods::*;

const MIGRATION_WASM: &[u8] =
    include_bytes!("../../../tests/fixtures/synthetic/module_migration/migration.wasm");

fn stage() -> StagedPackage {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("migration.wasm"), MIGRATION_WASM).unwrap();
    fs::write(
        temp.path().join("tantalus-package.json"),
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "package_id": "fixture.migration",
            "version": "2.0.0",
            "save_schema": 2,
            "entrypoints": [{"name": "migrate", "source_path": "migration.wasm", "abi_version": 1}],
            "world": {"spawn_regions": []},
            "records": []
        }))
        .unwrap(),
    )
    .unwrap();
    PackageLoader::stage(temp.path()).unwrap()
}

fn profile(staged: &StagedPackage, schema: u32) -> ProfileLock {
    ProfileLock {
        content_revision: ContentRevision::from_u64(1),
        content_fingerprint: staged.content_hash.clone(),
        modules: vec![ModuleLock {
            package: "fixture.migration".to_owned(),
            version: "2.0.0".to_owned(),
            required: true,
            state_schema: schema,
            content_hash: staged.content_hash.clone(),
            features: Vec::new(),
        }],
    }
}

fn source(staged: &StagedPackage, schema: u32) -> SaveEnvelope {
    SaveEnvelope {
        format_version: CURRENT_SAVE_FORMAT_VERSION,
        engine: EngineIdentity {
            name: "tantalus".to_owned(),
            version: "0.1.0".to_owned(),
        },
        ruleset: ContentId::parse("fixture:rules/default").unwrap(),
        profile: profile(staged, 1),
        checkpoint: CheckpointMetadata {
            kind: CheckpointKind::Progress,
            tick: Tick::new(1),
            resume_providers: Vec::new(),
        },
        state: AuthoritativeState::default(),
        modules: BTreeMap::from([(
            ContentId::parse("fixture:state/migration").unwrap(),
            ModuleSaveSection {
                schema_version: schema,
                state: json!({"value": 9}),
            },
        )]),
        migration_history: Vec::new(),
        unknown_sections: Vec::new(),
        outcomes: Vec::new(),
    }
}

#[test]
fn external_wasm_migration_updates_only_the_cloned_section_and_profile() {
    assert_eq!(
        ModuleHost::wasm_sha256(MIGRATION_WASM),
        include_str!("../../../tests/fixtures/synthetic/module_migration/migration.wasm.sha256")
            .trim()
    );
    let staged = stage();
    let source = source(&staged, 1);
    let target = profile(&staged, 2);
    let migrated = ModuleMigration {
        target_profile: &target,
        staged: &staged,
        entrypoint: "migrate",
        section: &ContentId::parse("fixture:state/migration").unwrap(),
        limits: ModuleLimits {
            fuel: 5_000_000,
            ..ModuleLimits::default()
        },
    }
    .migrate(&source)
    .unwrap();

    assert_eq!(
        source.modules[&ContentId::parse("fixture:state/migration").unwrap()].state,
        json!({"value": 9})
    );
    assert_eq!(source.profile.modules[0].state_schema, 1);
    assert!(source.migration_history.is_empty());
    let section = &migrated.modules[&ContentId::parse("fixture:state/migration").unwrap()];
    assert_eq!(section.schema_version, 2);
    assert_eq!(
        section.state,
        json!({"value": 9, "migrated": true, "schema": 2})
    );
    assert_eq!(migrated.profile, target);
    assert_eq!(migrated.migration_history.len(), 1);
    assert_eq!(migrated.migration_history[0].owner, "fixture.migration");
}

#[test]
fn trapped_module_migration_preserves_the_source_envelope() {
    let staged = stage();
    let source = source(&staged, 0);
    let before = serde_json::to_vec(&source).unwrap();
    let target = profile(&staged, 2);
    let error = ModuleMigration {
        target_profile: &target,
        staged: &staged,
        entrypoint: "migrate",
        section: &ContentId::parse("fixture:state/migration").unwrap(),
        limits: ModuleLimits {
            fuel: 5_000_000,
            ..ModuleLimits::default()
        },
    }
    .migrate(&source)
    .unwrap_err();
    assert_eq!(error.kind, ModuleErrorKind::Trap);
    assert_eq!(serde_json::to_vec(&source).unwrap(), before);
}
