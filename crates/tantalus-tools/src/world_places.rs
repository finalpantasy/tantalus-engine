//! World map location entry points, read from the world script.
//!
//! The foot actor raises `WorldEvent(x, z, id)` when the cell under it has an
//! event id (`ff9.cs:5078`): the engine requests function
//! `0x8000 | z << 8 & 0x3F00 | x << 2 & 0xFC | id & 3` on the world script's
//! first object. Each of those handlers sets map variable 39 to the place the
//! cell leads to and requests the enter-the-place function (`EVT_WORLD_WORLD00`
//! entry 0, functions above 0x8000).

use std::collections::BTreeMap;
use std::path::Path;

use tantalus_legacy::{ExpressionOp, InstructionKind, VariableScope};

use crate::field_session::{decode_pool, load_event};

/// The event function id `ff9.WorldEvent` requests.
pub fn event_code(x: i32, z: i32, id: i32) -> u16 {
    (0x8000 | (z << 8 & 0x3F00) | (x << 2 & 0xFC) | (id & 3)) as u16
}

/// Event function id to the place number it hands to the enter script
/// (the first constant its handler writes to map variable 39).
pub fn event_places(bundle: &Path, name: &str) -> Result<BTreeMap<u16, i32>, String> {
    let binary = load_event(bundle, &format!("{name}.eb"))?;
    let (pool, _) = decode_pool(&binary)?;
    let mut places = BTreeMap::new();
    let Some(entry) = pool.first() else {
        return Ok(places);
    };
    for (function, program) in entry {
        let code = *function;
        if code < 0x8000 {
            continue;
        }
        let place = program.instructions.values().find_map(|instruction| {
            let InstructionKind::Evaluate(expression) = &instruction.kind else {
                return None;
            };
            match expression.0.as_slice() {
                [
                    ExpressionOp::Variable(variable),
                    ExpressionOp::Constant(value),
                    ExpressionOp::Assign,
                    ..,
                ] if variable.scope == VariableScope::Map && variable.index == 39 => Some(*value),
                _ => None,
            }
        });
        if let Some(place) = place {
            places.insert(code, place);
        }
    }
    Ok(places)
}
