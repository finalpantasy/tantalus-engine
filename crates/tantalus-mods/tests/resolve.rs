use std::collections::BTreeMap;
use tantalus_contracts::*;
use tantalus_core::SaveStore;
use tantalus_mods::*;

fn dependency(package: &str, range: &str) -> PackageDependency {
    PackageDependency {
        package: package.to_owned(),
        version_range: range.to_owned(),
        optional: false,
    }
}

fn candidate(package: &str, version: &str) -> PackageCandidate {
    PackageCandidate {
        manifest: PackageManifest {
            schema_version: 1,
            package_id: package.to_owned(),
            version: version.to_owned(),
            engine_range: "*".to_owned(),
            sdk_range: "*".to_owned(),
            save_schema: 0,
            dependencies: Vec::new(),
            features: Vec::new(),
            providers: Vec::new(),
            entrypoints: Vec::new(),
            world: WorldPackage::default(),
            records: Vec::new(),
        },
        content_hash: format!("{:0>64}", package.replace('.', "_")),
    }
}

fn request(available: Vec<PackageCandidate>, requested: &[&str]) -> ProfileRequest {
    ProfileRequest {
        engine_version: "0.1.0".to_owned(),
        sdk_version: "1.0.0".to_owned(),
        base_revision: ContentRevision::from_u64(9),
        available,
        requested: requested
            .iter()
            .map(|package| (*package).to_owned())
            .collect(),
        enabled_features: BTreeMap::new(),
        load_order: Vec::new(),
    }
}

fn with_dependency(
    mut candidate: PackageCandidate,
    package: &str,
    range: &str,
) -> PackageCandidate {
    candidate
        .manifest
        .dependencies
        .push(dependency(package, range));
    candidate
}

fn content_record(id: &str, operation: PackageOperation) -> PackageRecord {
    PackageRecord {
        id: ContentId::parse(id).unwrap(),
        source_path: "record.json".to_owned(),
        operation,
    }
}

#[test]
fn dependencies_load_first_and_lock_exact_versions_and_hashes() {
    let root = with_dependency(candidate("fixture.app", "1.2.0"), "fixture.core", ">=1, <3");
    let resolved = ModResolver::resolve(&request(
        vec![
            candidate("fixture.core", "1.9.0"),
            root,
            candidate("fixture.core", "2.0.0"),
        ],
        &["fixture.app"],
    ))
    .unwrap();

    assert_eq!(
        resolved
            .lock
            .modules
            .iter()
            .map(|module| module.package.as_str())
            .collect::<Vec<_>>(),
        ["fixture.core", "fixture.app"]
    );
    assert_eq!(resolved.lock.modules[0].version, "2.0.0");
    assert_eq!(resolved.lock.modules[0].content_hash.len(), 64);
    assert_eq!(resolved.lock.content_revision, ContentRevision::from_u64(9));
    assert_eq!(resolved.lock.content_fingerprint.len(), 64);
}

#[test]
fn resolver_backtracks_when_a_later_dependency_rejects_the_newest_version() {
    let a = with_dependency(candidate("fixture.a", "1.0.0"), "fixture.shared", ">=1, <3");
    let b = with_dependency(candidate("fixture.b", "1.0.0"), "fixture.shared", "<2");
    let resolved = ModResolver::resolve(&request(
        vec![
            a,
            candidate("fixture.shared", "2.0.0"),
            candidate("fixture.shared", "1.5.0"),
            b,
        ],
        &["fixture.a", "fixture.b"],
    ))
    .unwrap();

    assert!(
        resolved
            .lock
            .modules
            .iter()
            .any(|module| module.package == "fixture.shared" && module.version == "1.5.0")
    );
}

#[test]
fn missing_packages_cycles_and_conflicts_have_specific_errors() {
    let missing = ModResolver::resolve(&request(Vec::new(), &["fixture.absent"])).unwrap_err();
    assert_eq!(missing.code, ResolutionErrorCode::UnknownPackage);

    let root = with_dependency(candidate("fixture.app", "1.0.0"), "fixture.core", "=9.0.0");
    let missing_dependency =
        ModResolver::resolve(&request(vec![root], &["fixture.app"])).unwrap_err();
    assert_eq!(
        missing_dependency.code,
        ResolutionErrorCode::MissingDependency
    );

    let a = with_dependency(candidate("fixture.a", "1.0.0"), "fixture.b", "*");
    let b = with_dependency(candidate("fixture.b", "1.0.0"), "fixture.a", "*");
    let cycle = ModResolver::resolve(&request(vec![a, b], &["fixture.a"])).unwrap_err();
    assert_eq!(cycle.code, ResolutionErrorCode::DependencyCycle);

    let conflict = with_dependency(candidate("fixture.app", "1.0.0"), "fixture.core", ">3");
    let conflict = ModResolver::resolve(&request(
        vec![conflict, candidate("fixture.core", "2.0.0")],
        &["fixture.app"],
    ))
    .unwrap_err();
    assert_eq!(conflict.code, ResolutionErrorCode::VersionConflict);
}

#[test]
fn engine_sdk_and_dependency_ranges_are_validated() {
    let mut incompatible = candidate("fixture.old", "1.0.0");
    incompatible.manifest.engine_range = ">=2".to_owned();
    let error = ModResolver::resolve(&request(vec![incompatible], &["fixture.old"])).unwrap_err();
    assert_eq!(error.code, ResolutionErrorCode::IncompatibleEngine);

    let mut incompatible = candidate("fixture.sdk", "1.0.0");
    incompatible.manifest.sdk_range = ">=2".to_owned();
    let error = ModResolver::resolve(&request(vec![incompatible], &["fixture.sdk"])).unwrap_err();
    assert_eq!(error.code, ResolutionErrorCode::IncompatibleSdk);

    let malformed = with_dependency(
        candidate("fixture.bad", "1.0.0"),
        "fixture.core",
        "not semver",
    );
    let error = ModResolver::resolve(&request(vec![malformed], &["fixture.bad"])).unwrap_err();
    assert_eq!(error.code, ResolutionErrorCode::InvalidRequest);
}

#[test]
fn feature_dependencies_are_explicit_and_optional_dependencies_stay_inactive() {
    let mut root = candidate("fixture.app", "1.0.0");
    root.manifest.dependencies.push(PackageDependency {
        package: "fixture.unused".to_owned(),
        version_range: "*".to_owned(),
        optional: true,
    });
    root.manifest.features.push(PackageFeature {
        name: "cards".to_owned(),
        dependencies: vec![dependency("fixture.cards", "^2")],
    });
    root.manifest.save_schema = 4;
    let mut req = request(
        vec![root, candidate("fixture.cards", "2.1.0")],
        &["fixture.app"],
    );
    req.enabled_features
        .insert("fixture.app".to_owned(), vec!["cards".to_owned()]);

    let resolved = ModResolver::resolve(&req).unwrap();
    assert_eq!(resolved.lock.modules.len(), 2);
    assert_eq!(resolved.lock.modules[0].package, "fixture.cards");
    assert_eq!(resolved.lock.modules[1].features, ["cards"]);
    assert_eq!(resolved.lock.modules[1].state_schema, 4);
    assert!(
        !resolved
            .lock
            .modules
            .iter()
            .any(|module| module.package == "fixture.unused")
    );

    req.enabled_features
        .insert("fixture.app".to_owned(), vec!["not_a_feature".to_owned()]);
    assert_eq!(
        ModResolver::resolve(&req).unwrap_err().code,
        ResolutionErrorCode::InvalidFeature
    );
}

#[test]
fn duplicate_additions_fail_but_declared_replacements_remain_ordered() {
    let mut first = candidate("fixture.first", "1.0.0");
    first
        .manifest
        .records
        .push(content_record("fixture:item/shared", PackageOperation::Add));
    let mut second = candidate("fixture.second", "1.0.0");
    second
        .manifest
        .records
        .push(content_record("fixture:item/shared", PackageOperation::Add));
    let error = ModResolver::resolve(&request(
        vec![first.clone(), second.clone()],
        &["fixture.first", "fixture.second"],
    ))
    .unwrap_err();
    assert_eq!(error.code, ResolutionErrorCode::DuplicateContent);

    second.manifest.records[0].operation = PackageOperation::Replace;
    let mut third = candidate("fixture.third", "1.0.0");
    third.manifest.records.push(content_record(
        "fixture:item/shared",
        PackageOperation::Replace,
    ));
    let mut req = request(
        vec![first, second, third],
        &["fixture.first", "fixture.second", "fixture.third"],
    );
    req.load_order = vec![
        "fixture.first".to_owned(),
        "fixture.second".to_owned(),
        "fixture.third".to_owned(),
    ];
    let resolved = ModResolver::resolve(&req).unwrap();
    assert_eq!(
        resolved
            .lock
            .modules
            .iter()
            .map(|module| module.package.as_str())
            .collect::<Vec<_>>(),
        ["fixture.first", "fixture.second", "fixture.third"]
    );
}

#[test]
fn exclusive_providers_conflict_and_shared_providers_keep_load_order() {
    let service = ContentId::parse("fixture:service/combat").unwrap();
    let mut first = candidate("fixture.first", "1.0.0");
    first.manifest.providers.push(ProviderDeclaration {
        service: service.clone(),
        exclusive: true,
        entrypoint: None,
    });
    let mut second = candidate("fixture.second", "1.0.0");
    second.manifest.providers.push(ProviderDeclaration {
        service: service.clone(),
        exclusive: false,
        entrypoint: None,
    });
    let error = ModResolver::resolve(&request(
        vec![first.clone(), second.clone()],
        &["fixture.first", "fixture.second"],
    ))
    .unwrap_err();
    assert_eq!(error.code, ResolutionErrorCode::ProviderConflict);

    first.manifest.providers[0].exclusive = false;
    let mut req = request(vec![second, first], &["fixture.second", "fixture.first"]);
    req.load_order = vec!["fixture.second".to_owned(), "fixture.first".to_owned()];
    let resolved = ModResolver::resolve(&req).unwrap();
    assert_eq!(
        resolved.providers[0].packages,
        ["fixture.second", "fixture.first"]
    );
}

#[test]
fn available_order_does_not_change_the_lock() {
    let a = candidate("fixture.a", "1.0.0");
    let b = candidate("fixture.b", "1.0.0");
    let left = ModResolver::resolve(&request(
        vec![a.clone(), b.clone()],
        &["fixture.a", "fixture.b"],
    ))
    .unwrap();
    let right = ModResolver::resolve(&request(vec![b, a], &["fixture.a", "fixture.b"])).unwrap();
    assert_eq!(left, right);
}

fn minimal_envelope(lock: ProfileLock) -> SaveEnvelope {
    SaveEnvelope {
        format_version: CURRENT_SAVE_FORMAT_VERSION,
        engine: EngineIdentity {
            name: "tantalus".to_owned(),
            version: "0.1.0".to_owned(),
        },
        ruleset: ContentId::parse("fixture:rules/default").unwrap(),
        profile: lock,
        checkpoint: CheckpointMetadata {
            kind: CheckpointKind::Progress,
            tick: Tick::new(1),
            resume_providers: Vec::new(),
        },
        state: AuthoritativeState::default(),
        modules: BTreeMap::new(),
        migration_history: Vec::new(),
        unknown_sections: Vec::new(),
        outcomes: Vec::new(),
    }
}

#[test]
fn resolved_lock_round_trips_through_native_save_and_blocks_missing_module() {
    let resolved = ModResolver::resolve(&request(
        vec![candidate("fixture.app", "1.0.0")],
        &["fixture.app"],
    ))
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut store = SaveStore::new(temp.path());
    let id = store
        .commit(&minimal_envelope(resolved.lock.clone()))
        .unwrap();
    let loaded = store
        .load(id, &resolved.lock)
        .unwrap()
        .into_ready()
        .unwrap();
    assert_eq!(loaded.profile, resolved.lock);

    let missing = ProfileLock {
        modules: Vec::new(),
        ..resolved.lock
    };
    let plan = store.load(id, &missing).unwrap();
    assert_eq!(plan.missing_required_modules, ["fixture.app"]);
    assert!(plan.into_ready().is_err());
}
