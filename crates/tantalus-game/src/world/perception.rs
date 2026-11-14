use super::{BehaviorDefinition, PlayerObservation};
use tantalus_contracts::Point3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Perception {
    pub noticed_player: bool,
    pub within_encounter_range: bool,
}

pub fn perceive(
    actor: Point3,
    player: &PlayerObservation,
    behavior: &BehaviorDefinition,
) -> Perception {
    let distance = distance_squared(actor, player.position);
    let sight = u128::from(behavior.sight_range).pow(2);
    let hearing = u128::from(behavior.hearing_range).pow(2);
    let encounter = u128::from(behavior.encounter_range).pow(2);
    Perception {
        noticed_player: (player.visible && distance <= sight)
            || (!player.visible && distance <= hearing),
        within_encounter_range: distance <= encounter,
    }
}

pub fn move_toward(current: Point3, target: Point3, maximum_step: u32) -> Point3 {
    let dx = i128::from(target.x) - i128::from(current.x);
    let dy = i128::from(target.y) - i128::from(current.y);
    let dz = i128::from(target.z) - i128::from(current.z);
    let distance = integer_sqrt((dx * dx + dy * dy + dz * dz) as u128);
    if distance == 0 || distance <= u128::from(maximum_step) {
        return target;
    }

    let step = i128::from(maximum_step);
    let distance = distance as i128;
    Point3 {
        x: (i128::from(current.x) + dx * step / distance) as i32,
        y: (i128::from(current.y) + dy * step / distance) as i32,
        z: (i128::from(current.z) + dz * step / distance) as i32,
    }
}

pub fn flee_direction(actor: Point3, player: Point3) -> Point3 {
    Point3 {
        x: actor.x + (actor.x - player.x),
        y: actor.y + (actor.y - player.y),
        z: actor.z + (actor.z - player.z),
    }
}

fn distance_squared(left: Point3, right: Point3) -> u128 {
    let x = i128::from(left.x) - i128::from(right.x);
    let y = i128::from(left.y) - i128::from(right.y);
    let z = i128::from(left.z) - i128::from(right.z);
    (x * x + y * y + z * z) as u128
}

fn integer_sqrt(value: u128) -> u128 {
    if value == 0 {
        return 0;
    }
    let mut current = value;
    let mut next = current.div_ceil(2);
    while next < current {
        current = next;
        next = (current + value / current) / 2;
    }
    current
}
