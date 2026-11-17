//! Memoria's per-field script patches for the vanilla game, keyed the way the reference keys
//! them (map number, scenario counter, object entry, original arguments). Each rewrites one
//! event before the host applies it. Source: `EventEngine.DoEventCode.cs`, opcodes `POS`/`DPOS`
//! (:278), `MOVE` (:636) and `POS3`/`DPOS3` (:2133). Ledger rows: `memoria-field-script-*`.

use super::world::FieldWorld;
use tantalus_legacy::{HostEvent, VariableType};

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
