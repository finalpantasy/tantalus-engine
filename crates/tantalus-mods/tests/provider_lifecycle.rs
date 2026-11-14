use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use tantalus_contracts::{ContentId, ContentRevision};
use tantalus_mods::*;

const CARD_WASM: &[u8] =
    include_bytes!("../../../tests/fixtures/synthetic/card_mode/card_mode.wasm");

fn service() -> ContentId {
    ContentId::parse("fixture:service/minigame_cards").unwrap()
}

fn staged() -> StagedPackage {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("card.wasm"), CARD_WASM).unwrap();
    fs::write(
        temp.path().join("tantalus-package.json"),
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "package_id": "fixture.card",
            "version": "1.0.0",
            "save_schema": 1,
            "providers": [{
                "service": "fixture:service/minigame_cards",
                "exclusive": true,
                "entrypoint": "card"
            }],
            "entrypoints": [{"name": "card", "source_path": "card.wasm", "abi_version": 1}],
            "world": {"spawn_regions": []},
            "records": []
        }))
        .unwrap(),
    )
    .unwrap();
    PackageLoader::stage(temp.path()).unwrap()
}

fn profile(staged: &StagedPackage) -> ResolvedProfile {
    ModResolver::resolve(&ProfileRequest {
        engine_version: "0.1.0".to_owned(),
        sdk_version: "1.0.0".to_owned(),
        base_revision: ContentRevision::from_u64(1),
        available: vec![PackageCandidate::from(staged.clone())],
        requested: vec!["fixture.card".to_owned()],
        enabled_features: BTreeMap::new(),
        load_order: Vec::new(),
    })
    .unwrap()
}

fn registry() -> ProviderRegistry {
    let staged = staged();
    ProviderRegistry::instantiate(
        &profile(&staged),
        &[staged],
        ModuleLimits {
            fuel: 5_000_000,
            ..ModuleLimits::default()
        },
    )
    .unwrap()
}

fn play() -> Value {
    json!({
        "match_id": 12,
        "rounds": 2,
        "player_card": {"instance": 100, "definition": "fixture:card/player", "power": 5},
        "ai_cards": [{"instance": 22, "definition": "fixture:card/ai", "power": 4}]
    })
}

#[test]
fn provider_initializes_dispatches_snapshots_and_shuts_down() {
    let profile = profile(&staged());
    assert_eq!(profile.providers[0].bindings[0].entrypoint, "card");
    let mut registry = registry();
    assert_eq!(registry.phase(&service()), Some(ProviderPhase::Running));

    let result = registry.dispatch(&service(), &play()).unwrap();
    assert_eq!(result.module_state["player_score"], json!(1));
    assert_eq!(registry.snapshot(&service()).unwrap()["round"], json!(1));

    registry.shutdown(&service()).unwrap();
    assert_eq!(registry.phase(&service()), Some(ProviderPhase::Stopped));
    assert!(registry.dispatch(&service(), &play()).is_err());
    assert!(registry.snapshot(&service()).is_err());
}

#[test]
fn provider_failure_blocks_further_dispatch_without_fallback() {
    let mut registry = registry();
    let error = registry
        .dispatch(&service(), &json!({"trap": true}))
        .unwrap_err();
    assert_eq!(error.kind, ModuleErrorKind::Trap);
    assert_eq!(registry.phase(&service()), Some(ProviderPhase::Failed));

    let error = registry.dispatch(&service(), &play()).unwrap_err();
    assert_eq!(error.kind, ModuleErrorKind::HostValidation);
    assert!(error.context.contains("no fallback"));
}
