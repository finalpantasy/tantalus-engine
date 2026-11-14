use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tantalus_sdk::{ABI_VERSION, emit_json, input_bytes, output_bytes};

#[derive(Debug, Deserialize)]
struct Request {
    abi_version: i32,
    module_state: Option<CardState>,
    input: Value,
}

#[derive(Debug, Deserialize)]
struct MatchInput {
    match_id: u64,
    rounds: u32,
    player_card: Card,
    ai_cards: Vec<Card>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Card {
    instance: u64,
    definition: String,
    power: i32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CardState {
    round: u32,
    player_score: u32,
    ai_score: u32,
    finished: bool,
    rewarded: bool,
}

#[derive(Debug, Serialize)]
struct GuestOutput {
    state: CardState,
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
    if input.get("trap").and_then(Value::as_bool) == Some(true) {
        panic!("requested provider failure");
    }
    if let Some(operation) = input.get("operation").and_then(Value::as_str) {
        assert!(matches!(operation, "initialize" | "shutdown"));
        return output_bytes(
            serde_json::to_vec(&GuestOutput {
                state,
                events: Vec::new(),
            })
            .unwrap(),
        );
    }
    let input: MatchInput = serde_json::from_value(input).unwrap();
    let mut events = vec![json!({
        "name": "card.ui.update",
        "data": {"match": input.match_id, "round": state.round + 1}
    })];

    if !state.finished {
        let ai_card = input
            .ai_cards
            .into_iter()
            .max_by_key(|card| (card.power, card.instance))
            .unwrap();
        events.push(json!({
            "name": "card.played",
            "data": {"owner": "player", "card": input.player_card}
        }));
        events.push(json!({
            "name": "ai.selected",
            "data": {"owner": "ai", "card": ai_card}
        }));
        match input.player_card.power.cmp(&ai_card.power) {
            std::cmp::Ordering::Greater => state.player_score += 1,
            std::cmp::Ordering::Less => state.ai_score += 1,
            std::cmp::Ordering::Equal => {}
        }
        state.round += 1;
        state.finished = state.round >= input.rounds;
        events.push(json!({
            "name": "round.resolved",
            "data": {"player": state.player_score, "ai": state.ai_score}
        }));
        emit(json!({
            "kind": "set_state",
            "key": "fixture:cards/match",
            "value": {"round": state.round, "player": state.player_score, "ai": state.ai_score}
        }));
    }

    if state.finished && !state.rewarded && state.player_score > state.ai_score {
        state.rewarded = true;
        emit(json!({"kind": "allocate_id"}));
        emit(json!({
            "kind": "set_state",
            "key": "fixture:reward/card_champion",
            "value": {"definition": "fixture:card/champion", "match": input.match_id}
        }));
        emit(json!({
            "kind": "emit_event",
            "name": "progression.card_win",
            "data": {"match": input.match_id}
        }));
        events.push(json!({"name": "match.completed", "data": {"winner": "player"}}));
    }

    output_bytes(serde_json::to_vec(&GuestOutput { state, events }).unwrap())
}

fn emit(value: Value) {
    unsafe { emit_json(&serde_json::to_vec(&value).unwrap()) }
}
