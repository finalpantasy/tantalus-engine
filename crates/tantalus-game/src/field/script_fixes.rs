//! Memoria's per-field script patches for the vanilla game, keyed the way the reference keys
//! them (map number, scenario counter, object entry, original arguments). Each rewrites one
//! event before the host applies it. Source: `EventEngine.DoEventCode.cs`, opcodes `POS`/`DPOS`
//! (:278), `MOVE` (:636), `MOVE3` (:833), `CFLAG` (:2029) and `POS3`/`DPOS3` (:2133), and
//! `EventEngine.MoveToward.cs`. Ledger rows: `memoria-field-script-*`.

use super::world::FieldWorld;
use tantalus_contracts::EntityId;
use tantalus_legacy::{
    EventProgram, Expression, ExpressionOp, HostEvent, InstructionKind, VariableRef, VariableScope,
    VariableType,
};

/// Memoria's `EBin.expr` skips (`EBin.cs:307`): `(map, entry, function ordinal, offset in
/// the function, the expression)`. When the object reaches the expression the reference moves
/// `ip` past it and returns, so the statement does nothing and the function goes on.
///
/// Field 705 (Gizamaluke/Bell Room), the female moogle (entry 3, loop function, entry ip 541,
/// that is function offset 92 + 448 + 1): after the Kupo Nut is given (map byte 24 = 15) she
/// would set the arrival counter map byte 26 to 2 in the tick Vivi's entry 6 set it to
/// `4 + Global.Byte[4]`. Entry 3 runs after entry 6 (creation order), so the 2 would stand and
/// the four arrivals (3, 6, 7, 10) take it past 0: the director advances while two actors
/// still wait, and they wait for `Byte[26] > 0` to end forever. Skipping it keeps 4.
const EXPRESSION_SKIPS: &[(i32, u64, u64, u32, VariableRef, i32)] = &[(
    705,
    3,
    1,
    448,
    VariableRef {
        scope: VariableScope::Map,
        value_type: VariableType::Byte,
        index: 26,
    },
    2,
)];

impl FieldWorld {
    /// `EBin.SC_COUNTER_SVR`: the first global word.
    fn scenario_counter(&self) -> i32 {
        self.global_memory.get(0, VariableType::UInt16).unwrap_or(0)
    }

    /// The patched copy of `event`, or `None` when Memoria leaves it alone.
    pub(crate) fn script_fix(&self, event: &HostEvent) -> Option<HostEvent> {
        let map = self.field_map_no;
        let sc = self.scenario_counter();
        match event {
            // `MOVE`: field 53, Blank jumps into the hole; destination X 250 becomes 330
            // (DoEventCode.cs:641, V01).
            HostEvent::WalkActor { actor, destination }
                if map == 53 && *destination == [250, 1200] =>
            {
                Some(HostEvent::WalkActor {
                    actor: *actor,
                    destination: [330, 1200],
                })
            }
            // `MOVE`: field 1815, Steiner on the dock walks further so he is out of the wide
            // frame (:724).
            HostEvent::WalkActor { actor, destination }
                if map == 1815 && sc == 7070 && *destination == [-2312, -245] =>
            {
                Some(HostEvent::WalkActor {
                    actor: *actor,
                    destination: [-3000, -245],
                })
            }
            // `MOVE`: field 1901, Eiko ATE, Quina chasing Gilmanesh runs far enough (:738);
            // `walk_flag_fix` hides them at the edge.
            HostEvent::WalkActor { actor, destination }
                if map == 1901 && matches!(actor.get(), 2 | 17) && destination[0] == 672 =>
            {
                Some(HostEvent::WalkActor {
                    actor: *actor,
                    destination: [1000, destination[1]],
                })
            }
            // `MOVE3` (mode 2): field 1550, Quina runs to eat the Mog (:833); and the
            // `MoveToward_mixed` patch for Dagger's mid-step (`MoveToward.cs:24`).
            HostEvent::WalkActorMixed {
                actor,
                destination,
                mode,
            } => {
                let [x, y, z] = *destination;
                let fixed = match (map, actor.get()) {
                    (1550, 18) if *mode == 2 && x == 1798 => [2800, y, z],
                    (1550, 2) if *mode == 2 && x == 1797 => [2800, y, z],
                    (1550, 15) if x == -1109 && z == -1014 => [-1155, y, -1070],
                    _ => return None,
                };
                Some(HostEvent::WalkActorMixed {
                    actor: *actor,
                    destination: fixed,
                    mode: *mode,
                })
            }
            // `POS`/`DPOS` (pathing on, height 32768).
            HostEvent::SetActorPosition { actor, position }
            | HostEvent::MoveActorXzy { actor, position }
                if position[1] == 32768 =>
            {
                let [x, y, z] = *position;
                let id = actor.get();
                let (x, z) = match (map, id) {
                    // Evil Forest/Nest, Steiner's position (fix #892, :322).
                    (257, 8) if sc == 2300 && (x, z) == (96, -736) => (30, -716),
                    // Fossil Roo/Mining Site, the Lindblum worker stops one step short so he
                    // stays off the blocked exit path (:302, #486).
                    (1421, 5) if (x, z) == (1510, -2331) => (1510, -2231),
                    // Eiko ATE: Gilmanesh appears between two columns (:349).
                    (1901, 2) if sc == 7550 && (x, z) == (142, 2172) => (400, z),
                    // Dagger and Mikoto appear in the frame of the wide view (:374, :378);
                    // Memoria gates these on its widescreen setting, which is the default here.
                    (2600, 8) if sc == 10700 && (x, z) == (1173, -1685) => (x, -1985),
                    (2651, 4) if sc == 10890 && (x, z) == (1449, -6844) => (1749, -7244),
                    _ => return None,
                };
                Some(rebuild(event, [x, y, z]))
            }
            // `POS3`/`DPOS3`: position is `[x, -arg2, arg3]`.
            HostEvent::SetActorPosition { actor, position }
            | HostEvent::MoveActorXzy { actor, position } => {
                let [x, y, z] = *position;
                let id = actor.get();
                let fixed = match (map, id) {
                    // Alexandria Castle/Garnet's Room, Dagger: height 1274 (:2160, #666).
                    (1207, 9) => [x, y, 1274],
                    // Desert Palace/Sanctum, Kuja moves right to keep Zidane's surprise
                    // (:2190, #860).
                    (2209, 4) if sc == 9520 && [x, y, z] == [1123, 98, 1067] => [1323, y, z],
                    _ => return None,
                };
                if fixed == *position {
                    return None;
                }
                Some(rebuild(event, fixed))
            }
            _ => None,
        }
    }
}

impl FieldWorld {
    /// Applies [`EXPRESSION_SKIPS`] to a function program being registered for this field:
    /// `key` is the director key (`entry * 256 + ordinal`, an extra object's in the high bits).
    /// The skipped statement becomes a `NoOp` of the same length, so the next instruction runs
    /// in the same tick as in the reference.
    pub(crate) fn patch_function(&self, key: EntityId, program: &mut EventProgram) {
        let low = key.get() & 0xffff;
        let (entry, ordinal) = (low / 256, low % 256);
        for &(map, skip_entry, skip_ordinal, offset, variable, value) in EXPRESSION_SKIPS {
            if self.field_map_no != map || entry != skip_entry || ordinal != skip_ordinal {
                continue;
            }
            let expected = Expression(vec![
                ExpressionOp::Variable(variable),
                ExpressionOp::Constant(value),
                ExpressionOp::Assign,
                ExpressionOp::End,
            ]);
            if let Some(instruction) = program.instructions.get_mut(&offset)
                && instruction.kind == InstructionKind::Evaluate(expected)
            {
                instruction.kind = InstructionKind::NoOp(0x05);
            }
        }
    }

    /// Whether Memoria drops `event` altogether. `CFLAG` in field 103 (Alexandria square,
    /// the jump rope): objects 6 to 17 are never hidden when the script asks for flags 14
    /// (:2029), so the girls and the crowd stay drawn when the wide view shows them.
    pub(crate) fn script_skips(&self, event: &HostEvent) -> bool {
        matches!(event, HostEvent::SetObjectFlags { object, flags }
            if self.field_map_no == 103 && *flags == 14 && (6..=17).contains(&object.get()))
    }

    /// The side effect of the field 1901 walk patch (:738): Quina and Eiko past the edge
    /// of the wide view are made invisible (`flags = (flags & -64) | 14`).
    pub(crate) fn script_walk_flags(&mut self, event: &HostEvent) {
        let HostEvent::WalkActor { actor, destination } = event else {
            return;
        };
        if self.field_map_no != 1901 || destination[0] != 672 {
            return;
        }
        let edge = match actor.get() {
            2 => 890.0,
            17 => 810.0,
            _ => return,
        };
        if let Some(state) = self.actors.get_mut(actor)
            && state.position[0] > edge
        {
            state.flags = (state.flags & !63) | 14;
        }
    }
}

fn rebuild(event: &HostEvent, position: [i32; 3]) -> HostEvent {
    match event {
        HostEvent::SetActorPosition { actor, .. } => HostEvent::SetActorPosition {
            actor: *actor,
            position,
        },
        HostEvent::MoveActorXzy { actor, .. } => HostEvent::MoveActorXzy {
            actor: *actor,
            position,
        },
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tantalus_legacy::Instruction;

    /// The 705 loop function's two statements around the skip: `Byte[26] = 2` (8 bytes at
    /// 448) and the `Wait(1)` after it.
    fn moogle_loop() -> EventProgram {
        let counter = Expression(vec![
            ExpressionOp::Variable(VariableRef {
                scope: VariableScope::Map,
                value_type: VariableType::Byte,
                index: 26,
            }),
            ExpressionOp::Constant(2),
            ExpressionOp::Assign,
            ExpressionOp::End,
        ]);
        let mut instructions = BTreeMap::new();
        for (offset, length, kind) in [
            (448, 8, InstructionKind::Evaluate(counter)),
            (456, 3, InstructionKind::Yield(0)),
        ] {
            instructions.insert(
                offset,
                Instruction {
                    offset,
                    length,
                    kind,
                },
            );
        }
        EventProgram {
            instructions,
            source_length: 459,
        }
    }

    fn patched(map: i32, key: u64) -> bool {
        let mut world = FieldWorld::new();
        world.field_map_no = map;
        let mut program = moogle_loop();
        world.patch_function(EntityId::new(key), &mut program);
        let skip = &program.instructions[&448];
        assert_eq!(
            skip.next_offset(),
            456,
            "the skip keeps the statement's length"
        );
        skip.kind == InstructionKind::NoOp(0x05)
    }

    #[test]
    fn bell_room_moogle_keeps_the_arrival_count() {
        // EBin.cs:307: map 705, sid 3, ip 541 (entry 3's loop function, offset 448).
        assert!(patched(705, 3 * 256 + 1));
        // The same entry's object created with another uid keeps the patch.
        assert!(patched(705, (9 << 16) | (3 * 256 + 1)));
        // Other fields, entries and functions run the statement.
        assert!(!patched(704, 3 * 256 + 1));
        assert!(!patched(705, 6 * 256 + 1));
        assert!(!patched(705, 3 * 256));
    }
}
