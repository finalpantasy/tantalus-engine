use serde_json::{Value, json};
use tantalus_contracts::{ContentRevision, ModuleLock, ProfileLock};
use tantalus_mods::*;

const ACTION_WASM: &[u8] =
    include_bytes!("../../../tests/fixtures/synthetic/action_mode/action_mode.wasm");

fn lock() -> ProfileLock {
    ProfileLock {
        content_revision: ContentRevision::from_u64(1),
        content_fingerprint: "fixture".to_owned(),
        modules: vec![ModuleLock {
            package: "fixture.action".to_owned(),
            version: "1.0.0".to_owned(),
            required: true,
            state_schema: 1,
            content_hash: "a".repeat(64),
            features: Vec::new(),
        }],
    }
}

fn frame(host: &ModuleHost, tick: u64, attack: bool) -> Value {
    let player_x = host
        .authoritative_state()
        .get("fixture:world/player_x")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let enemy = host
        .authoritative_state()
        .get("fixture:world/enemy")
        .cloned()
        .unwrap_or_else(|| json!({"id": 7, "x": 4, "hp": 2}));
    json!({
        "tick": tick,
        "move_x": 1,
        "attack": attack,
        "player_x": player_x,
        "enemies": [enemy]
    })
}

#[test]
fn independently_compiled_rust_action_module_runs_through_public_host() {
    assert!(ACTION_WASM.starts_with(b"\0asm"));
    assert_eq!(
        ModuleHost::wasm_sha256(ACTION_WASM),
        include_str!("../../../tests/fixtures/synthetic/action_mode/action_mode.wasm.sha256")
            .trim()
    );
    let limits = ModuleLimits {
        fuel: 5_000_000,
        ..ModuleLimits::default()
    };
    let mut host = ModuleHost::instantiate(&lock(), "fixture.action", ACTION_WASM, limits).unwrap();

    for tick in 1..=3 {
        let result = host.invoke(&frame(&host, tick, true)).unwrap();
        assert_eq!(result.events[0].name, "hud.update");
        assert_eq!(
            host.authoritative_state()["fixture:world/enemy"]["hp"],
            json!(1)
        );
        assert_eq!(result.module_state["cooldown"], json!(4 - tick));
        assert!(
            !host
                .authoritative_state()
                .contains_key("fixture:loot/raptor_fang")
        );
    }

    let fourth = host.invoke(&frame(&host, 4, true)).unwrap();
    assert_eq!(fourth.module_state["cooldown"], json!(0));
    let victory_input = frame(&host, 5, true);
    let victory = host.invoke(&victory_input).unwrap();
    assert_eq!(victory.module_state["cooldown"], json!(3));
    assert_eq!(victory.module_state["loot_spawned"], json!(true));
    assert_eq!(
        host.authoritative_state()["fixture:world/enemy"]["hp"],
        json!(0)
    );
    assert_eq!(
        host.authoritative_state()["fixture:loot/raptor_fang"]["affix"],
        json!("keen")
    );
    assert_eq!(host.next_id(), 2);
    assert!(
        victory
            .events
            .iter()
            .any(|event| event.name == "attack.hit")
    );
    assert!(
        victory
            .events
            .iter()
            .any(|event| event.name == "loot.spawned")
    );

    let after = host.invoke(&frame(&host, 6, true)).unwrap();
    assert_eq!(host.next_id(), 2);
    assert!(!after.events.iter().any(|event| event.name == "attack.hit"));
    assert!(
        !after
            .events
            .iter()
            .any(|event| event.name == "loot.spawned")
    );
}
