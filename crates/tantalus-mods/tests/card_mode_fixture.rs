use serde_json::{Value, json};
use std::collections::BTreeMap;
use tantalus_contracts::{ContentId, ContentRevision};
use tantalus_mods::*;

const CARD_WASM: &[u8] =
    include_bytes!("../../../tests/fixtures/synthetic/card_mode/card_mode.wasm");

fn resolved_card_profile() -> ResolvedProfile {
    let service = ContentId::parse("fixture:service/minigame_cards").unwrap();
    let candidate = PackageCandidate {
        manifest: PackageManifest {
            schema_version: 1,
            package_id: "fixture.card".to_owned(),
            version: "1.0.0".to_owned(),
            engine_range: "*".to_owned(),
            sdk_range: "*".to_owned(),
            save_schema: 1,
            dependencies: Vec::new(),
            features: Vec::new(),
            providers: vec![ProviderDeclaration {
                service,
                exclusive: true,
            }],
            entrypoints: Vec::new(),
            world: WorldPackage::default(),
            records: Vec::new(),
        },
        content_hash: ModuleHost::wasm_sha256(CARD_WASM),
    };
    ModResolver::resolve(&ProfileRequest {
        engine_version: "0.1.0".to_owned(),
        sdk_version: "1.0.0".to_owned(),
        base_revision: ContentRevision::from_u64(1),
        available: vec![candidate],
        requested: vec!["fixture.card".to_owned()],
        enabled_features: BTreeMap::new(),
        load_order: Vec::new(),
    })
    .unwrap()
}

fn card(instance: u64, definition: &str, power: i32) -> Value {
    json!({"instance": instance, "definition": definition, "power": power})
}

fn input(player_power: i32) -> Value {
    json!({
        "match_id": 12,
        "rounds": 2,
        "player_card": card(100, "fixture:card/player", player_power),
        "ai_cards": [card(21, "fixture:card/weak", 1), card(22, "fixture:card/strong", 4)]
    })
}

fn host() -> ModuleHost {
    let limits = ModuleLimits {
        fuel: 5_000_000,
        ..ModuleLimits::default()
    };
    ModuleHost::instantiate(
        &resolved_card_profile().lock,
        "fixture.card",
        CARD_WASM,
        limits,
    )
    .unwrap()
}

#[test]
fn external_card_provider_runs_rules_ai_match_rewards_and_ui() {
    assert!(CARD_WASM.starts_with(b"\0asm"));
    assert_eq!(
        ModuleHost::wasm_sha256(CARD_WASM),
        include_str!("../../../tests/fixtures/synthetic/card_mode/card_mode.wasm.sha256").trim()
    );
    let profile = resolved_card_profile();
    assert_eq!(profile.lock.modules.len(), 1);
    assert_eq!(profile.providers[0].packages, ["fixture.card"]);

    let mut host = host();
    let first = host.invoke(&input(5)).unwrap();
    assert_eq!(first.module_state["player_score"], json!(1));
    assert_eq!(first.module_state["ai_score"], json!(0));
    assert_eq!(first.module_state["finished"], json!(false));
    assert!(
        first
            .events
            .iter()
            .any(|event| event.name == "card.ui.update")
    );
    assert!(first.events.iter().any(|event| {
        event.name == "ai.selected" && event.data["card"]["instance"] == json!(22)
    }));
    assert!(
        !host
            .authoritative_state()
            .contains_key("fixture:reward/card_champion")
    );

    let second = host.invoke(&input(6)).unwrap();
    assert_eq!(second.module_state["player_score"], json!(2));
    assert_eq!(second.module_state["finished"], json!(true));
    assert_eq!(second.module_state["rewarded"], json!(true));
    assert_eq!(
        host.authoritative_state()["fixture:reward/card_champion"]["match"],
        json!(12)
    );
    assert_eq!(host.next_id(), 2);
    assert!(
        second
            .events
            .iter()
            .any(|event| event.name == "match.completed")
    );
    assert!(
        second
            .events
            .iter()
            .any(|event| event.name == "progression.card_win")
    );

    let repeated = host.invoke(&input(6)).unwrap();
    assert_eq!(host.next_id(), 2);
    assert_eq!(repeated.module_state["rewarded"], json!(true));
    assert!(
        !repeated
            .events
            .iter()
            .any(|event| event.name == "progression.card_win")
    );
}

#[test]
fn card_match_replays_identically_in_two_hosts() {
    let mut left = host();
    let mut right = host();
    for power in [5, 6, 6] {
        let left_result = left.invoke(&input(power)).unwrap();
        let right_result = right.invoke(&input(power)).unwrap();
        assert_eq!(left_result, right_result);
        assert_eq!(left.authoritative_state(), right.authoritative_state());
    }
}
