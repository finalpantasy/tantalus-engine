use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tantalus_sdk::{ABI_VERSION, emit_json, input_bytes, output_bytes};

#[derive(Debug, Deserialize)]
struct Request {
    abi_version: i32,
    module_state: Option<ActionState>,
    input: FrameInput,
}

#[derive(Debug, Deserialize)]
struct FrameInput {
    tick: u64,
    move_x: i32,
    attack: bool,
    player_x: i32,
    enemies: Vec<EnemyInput>,
}

#[derive(Debug, Deserialize)]
struct EnemyInput {
    id: u64,
    x: i32,
    hp: i32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ActionState {
    cooldown: u32,
    loot_spawned: bool,
}

#[derive(Debug, Serialize)]
struct GuestOutput {
    state: ActionState,
    events: Vec<Value>,
}

#[unsafe(no_mangle)]
pub extern "C" fn tantalus_abi_version() -> i32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn alloc(length: i32) -> i32 {
    let buffer = Vec::<u8>::with_capacity(length as usize);
    let pointer = buffer.as_ptr() as i32;
    std::mem::forget(buffer);
    pointer
}

#[unsafe(no_mangle)]
pub extern "C" fn tantalus_invoke(packed: i64) -> i64 {
    let request: Request = serde_json::from_slice(unsafe { input_bytes(packed) }).unwrap();
    assert_eq!(request.abi_version, ABI_VERSION);
    let mut state = request.module_state.unwrap_or_default();
    let input = request.input;
    let player_x = input.player_x + input.move_x;
    let mut events = vec![json!({
        "name": "hud.update",
        "data": {"tick": input.tick, "player_x": player_x}
    })];

    let mut enemy = input.enemies.into_iter().min_by_key(|enemy| enemy.id).unwrap();
    if input.attack && state.cooldown == 0 && enemy.hp > 0 {
        enemy.hp -= 1;
        state.cooldown = 3;
        events.push(json!({
            "name": "attack.hit",
            "data": {"target": enemy.id, "hp_after": enemy.hp}
        }));
    } else {
        state.cooldown = state.cooldown.saturating_sub(1);
    }
    if enemy.hp > 0 {
        enemy.x += (player_x - enemy.x).signum();
    }

    emit(json!({
        "kind": "set_state",
        "key": "fixture:world/player_x",
        "value": player_x
    }));
    emit(json!({
        "kind": "set_state",
        "key": "fixture:world/enemy",
        "value": {"id": enemy.id, "x": enemy.x, "hp": enemy.hp}
    }));
    if enemy.hp == 0 && !state.loot_spawned {
        state.loot_spawned = true;
        emit(json!({"kind": "allocate_id"}));
        emit(json!({
            "kind": "set_state",
            "key": "fixture:loot/raptor_fang",
            "value": {"definition": "fixture:weapon/raptor_fang", "affix": "keen"}
        }));
        emit(json!({
            "kind": "emit_event",
            "name": "loot.spawned",
            "data": {"definition": "fixture:weapon/raptor_fang"}
        }));
    }

    let output = GuestOutput { state, events };
    output_bytes(serde_json::to_vec(&output).unwrap())
}

fn emit(value: Value) {
    unsafe { emit_json(&serde_json::to_vec(&value).unwrap()) }
}
