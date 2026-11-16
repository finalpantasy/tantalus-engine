//! Which clip each battle unit plays, and where it stands, at a moment of a
//! [`BattleStage`]: idle, the command it runs, a hit, its death.
//!
//! Everything is read back from the stage record (its per-tick
//! [`StageEvent`]s), so a replay and a live battle animate alike.
//! Sources (Memoria):
//! - player motions: `BattlePlayerCharacter.PlayerMotionIndex` over the
//!   `BattleParameters.csv` row (`btl_mot.setMotion`); an attack runs
//!   `MP_RUN_TO_ATTACK`, `MP_ATTACK`, `MP_BACK`, `MP_ATK_TO_NORMAL`, a spell
//!   `MP_IDLE_TO_CHANT`, `MP_CHANT`, `MP_MAGIC`, an item `MP_ITEM1`, Defend
//!   `MP_IDLE_TO_DEF`, `MP_DEFENCE`; a ready member waits in `MP_IDLE_CMD`;
//!   a hit plays `MP_DAMAGE1`, a death `MP_DOWN_DISABLE`;
//! - enemy motions: `BattleUnit.cs:876-881` (`mot[0]` idle, `mot[2]` hit,
//!   `mot[3]`/`mot[4]` death) and, for an attack, the `Anim` codes of its
//!   `btlseq` sequence (`SeqExecAnim`), walking up first when the sequence
//!   has a `MoveToTarget`.
//!
//! Not matched: the sequence timing itself (waits, effects, the camera); a
//! command here spans the machine's stand-in `Timing` (effect after 40
//! ticks, done after 75), and the clips are placed around the effect tick.

use tantalus_tools::battle_stage::{BATTLE_TICKS_PER_SECOND, BattleStage, StageEvent, StageModel};

/// A clip as `(folder geo id, AnimationDB key)`.
pub type ClipKey = (u32, u32);

/// `BattlePlayerCharacter.PlayerMotionIndex` values used here.
pub mod mp {
    pub const IDLE_NORMAL: usize = 0;
    pub const DAMAGE1: usize = 2;
    pub const DISABLE: usize = 4;
    pub const DOWN_DISABLE: usize = 8;
    pub const IDLE_CMD: usize = 9;
    pub const IDLE_TO_DEF: usize = 12;
    pub const DEFENCE: usize = 13;
    pub const RUN: usize = 21;
    pub const RUN_TO_ATTACK: usize = 22;
    pub const ATTACK: usize = 23;
    pub const BACK: usize = 24;
    pub const ATK_TO_NORMAL: usize = 25;
    pub const IDLE_TO_CHANT: usize = 26;
    pub const CHANT: usize = 27;
    pub const MAGIC: usize = 28;
    pub const ITEM1: usize = 31;
}

/// Machine ticks from a command's start to its effect and to its end
/// (`field_battle::COMMAND_TIMING`).
pub const EFFECT_TICKS: f64 = 40.0;
pub const COMMAND_TICKS: f64 = 75.0;
/// How far short of its target an attacker stops.
const STRIKE_DISTANCE: f32 = 450.0;
/// Seconds of the run up to the target.
const RUN_SECONDS: f32 = 0.5;
/// Seconds a defeated enemy stays after its death clip.
const FADE_SECONDS: f32 = 0.4;
/// Seconds a damage number stays up.
pub const NUMBER_SECONDS: f64 = 1.2;

/// The clips a unit can play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Motions {
    /// Indexed by `PlayerMotionIndex`.
    Player(Vec<Option<ClipKey>>),
    Enemy {
        idle: Option<ClipKey>,
        hit: Option<ClipKey>,
        death: Option<ClipKey>,
        /// Clips of each attack sequence by attack number, and whether it
        /// walks up to its target.
        attacks: Vec<(Vec<ClipKey>, bool)>,
    },
}

impl Motions {
    pub fn idle(&self) -> Option<ClipKey> {
        match self {
            Self::Player(clips) => clips.get(mp::IDLE_NORMAL).copied().flatten(),
            Self::Enemy { idle, .. } => *idle,
        }
    }

    fn player(&self, index: usize) -> Option<ClipKey> {
        match self {
            Self::Player(clips) => clips.get(index).copied().flatten().or(self.idle()),
            Self::Enemy { .. } => self.idle(),
        }
    }

    fn hit(&self) -> Option<ClipKey> {
        match self {
            Self::Player(_) => self.player(mp::DAMAGE1),
            Self::Enemy { hit, .. } => hit.or(self.idle()),
        }
    }

    fn death(&self) -> Option<ClipKey> {
        match self {
            Self::Player(_) => self.player(mp::DOWN_DISABLE),
            Self::Enemy { death, .. } => *death,
        }
    }

    /// Every clip, for loading.
    pub fn all(&self) -> Vec<ClipKey> {
        match self {
            Self::Player(clips) => clips.iter().flatten().copied().collect(),
            Self::Enemy {
                idle,
                hit,
                death,
                attacks,
            } => [*idle, *hit, *death]
                .into_iter()
                .flatten()
                .chain(attacks.iter().flat_map(|(clips, _)| clips.iter().copied()))
                .collect(),
        }
    }
}

/// What a unit shows at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitMotion {
    pub clip: Option<ClipKey>,
    /// Seconds into `clip`.
    pub time: f32,
    /// Whether `time` wraps (idles, chants) or holds at the end.
    pub looping: bool,
    /// Added to the unit's battle position (an attacker's run).
    pub offset: [f32; 3],
    /// Gone (a defeated enemy after its death).
    pub hidden: bool,
}

/// One piece of a timeline.
#[derive(Debug, Clone, Copy)]
struct Segment {
    clip: Option<ClipKey>,
    start: f32,
    length: f32,
    looping: bool,
    /// Offset at the segment's start and end (lerped).
    from: [f32; 3],
    to: [f32; 3],
}

fn seconds_of(index: usize) -> f64 {
    index as f64 / BATTLE_TICKS_PER_SECOND
}

/// A command `unit` ran: start, effect time and its first target.
#[derive(Debug, Clone, Copy)]
struct Running {
    start: f64,
    effect: f64,
    cmd_no: u32,
    sub_no: i32,
    target: Option<usize>,
}

/// The newest command of `unit` started by `seconds`.
fn last_command(stage: &BattleStage, unit: usize, seconds: f64) -> Option<Running> {
    let last = ((seconds.max(0.0) * BATTLE_TICKS_PER_SECOND) as usize + 1).min(stage.frames.len());
    let (index, cmd_no, sub_no, target) =
        stage.frames[..last]
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, frame)| {
                frame.events.iter().find_map(|event| match event {
                    StageEvent::Command {
                        unit: who,
                        cmd_no,
                        sub_no,
                        targets,
                    } if *who == unit => Some((index, *cmd_no, *sub_no, targets.first().copied())),
                    _ => None,
                })
            })?;
    let effect = stage.frames[index..]
        .iter()
        .enumerate()
        .find_map(|(offset, frame)| {
            frame
                .events
                .iter()
                .any(|event| matches!(event, StageEvent::Effect { caster, .. } if *caster == unit))
                .then_some(index + offset)
        })
        .map_or(
            seconds_of(index) + EFFECT_TICKS / BATTLE_TICKS_PER_SECOND,
            seconds_of,
        );
    Some(Running {
        start: seconds_of(index),
        effect,
        cmd_no,
        sub_no,
        target,
    })
}

/// The newest event on `unit` by `seconds` that `pick` accepts, with its
/// time.
fn last_event(
    stage: &BattleStage,
    seconds: f64,
    pick: impl Fn(&StageEvent) -> bool,
) -> Option<(f64, StageEvent)> {
    let last = ((seconds.max(0.0) * BATTLE_TICKS_PER_SECOND) as usize + 1).min(stage.frames.len());
    stage.frames[..last]
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, frame)| {
            frame
                .events
                .iter()
                .find(|event| pick(event))
                .map(|event| (seconds_of(index), event.clone()))
        })
}

/// Where an attacker on `from` stops in front of `target`, as an offset.
fn strike_offset(from: [f32; 3], target: [f32; 3]) -> [f32; 3] {
    let (dx, dz) = (target[0] - from[0], target[2] - from[2]);
    let distance = dx.hypot(dz);
    if distance <= STRIKE_DISTANCE {
        return [0.0; 3];
    }
    let scale = (distance - STRIKE_DISTANCE) / distance;
    [dx * scale, 0.0, dz * scale]
}

/// Unit `unit` at `seconds`. `length` gives a clip's length in seconds.
pub fn unit_motion(
    stage: &BattleStage,
    unit: usize,
    seconds: f64,
    motions: &Motions,
    length: &dyn Fn(ClipKey) -> f32,
) -> UnitMotion {
    let idle = UnitMotion {
        clip: motions.idle(),
        time: seconds as f32,
        looping: true,
        offset: [0.0; 3],
        hidden: false,
    };
    let len = |clip: Option<ClipKey>| clip.map_or(0.0, length);
    let record = stage.frame_at(seconds);
    let hp = record.and_then(|frame| frame.hp.get(unit).copied());
    let player = matches!(stage.units[unit].model, StageModel::Player { .. });
    // A death holds its clip; an enemy then goes.
    if hp == Some(0)
        && let Some((died, _)) = last_event(
            stage,
            seconds,
            |event| matches!(event, StageEvent::Died { unit: who } if *who == unit),
        )
    {
        let clip = motions.death();
        let since = (seconds - died) as f32;
        let total = len(clip);
        if !player && since > total + FADE_SECONDS {
            return UnitMotion {
                hidden: true,
                ..idle
            };
        }
        if player && since > total {
            return UnitMotion {
                clip: motions.player(mp::DISABLE),
                time: since - total,
                looping: true,
                ..idle
            };
        }
        return UnitMotion {
            clip,
            time: since.min(total),
            looping: false,
            ..idle
        };
    }
    if let Some(running) = last_command(stage, unit, seconds)
        && seconds < running.start + COMMAND_TICKS / BATTLE_TICKS_PER_SECOND
    {
        let segments = command_segments(stage, unit, running, motions, &len);
        let now = seconds as f32;
        if let Some(segment) = segments
            .iter()
            .rev()
            .find(|segment| now >= segment.start && now < segment.start + segment.length)
        {
            let t = if segment.length > 0.0 {
                (now - segment.start) / segment.length
            } else {
                1.0
            };
            let offset = [0, 1, 2]
                .map(|axis| segment.from[axis] + (segment.to[axis] - segment.from[axis]) * t);
            return UnitMotion {
                clip: segment.clip,
                time: now - segment.start,
                looping: segment.looping,
                offset,
                hidden: false,
            };
        }
    }
    if let Some((hit, _)) = last_event(
        stage,
        seconds,
        |event| matches!(event, StageEvent::Effect { target, damage, .. } if *target == unit && *damage > 0),
    ) {
        let clip = motions.hit();
        let since = (seconds - hit) as f32;
        if since < len(clip) {
            return UnitMotion {
                clip,
                time: since,
                looping: false,
                ..idle
            };
        }
    }
    let ready = record
        .and_then(|frame| frame.atb.get(unit).copied())
        .is_some_and(|(current, max)| max > 0 && current >= max);
    if player && ready {
        return UnitMotion {
            clip: motions.player(mp::IDLE_CMD),
            ..idle
        };
    }
    idle
}

/// The timeline of one command.
fn command_segments(
    stage: &BattleStage,
    unit: usize,
    running: Running,
    motions: &Motions,
    len: &dyn Fn(Option<ClipKey>) -> f32,
) -> Vec<Segment> {
    let home = stage.units[unit].position;
    let target = running
        .target
        .and_then(|target| stage.units.get(target))
        .map(|target| target.position);
    let strike = target.map_or([0.0; 3], |target| strike_offset(home, target));
    let start = running.start as f32;
    let effect = running.effect as f32;
    let still = |clip: Option<ClipKey>, start: f32, length: f32, looping: bool| Segment {
        clip,
        start,
        length,
        looping,
        from: [0.0; 3],
        to: [0.0; 3],
    };
    let mut out = Vec::new();
    match motions {
        Motions::Player(_) => match running.cmd_no {
            // Attack: run up, strike at the effect, jump back.
            1 => {
                let attack = motions.player(mp::ATTACK);
                let lead_in = motions.player(mp::RUN_TO_ATTACK);
                let strike_at = effect - len(Some(attack).flatten()) * 0.5;
                let lead_at = strike_at - len(lead_in);
                let run_at = (lead_at - RUN_SECONDS).max(start);
                out.push(Segment {
                    clip: motions.player(mp::RUN),
                    start: run_at,
                    length: lead_at - run_at,
                    looping: true,
                    from: [0.0; 3],
                    to: strike,
                });
                out.push(Segment {
                    from: strike,
                    to: strike,
                    ..still(lead_in, lead_at, len(lead_in), false)
                });
                out.push(Segment {
                    from: strike,
                    to: strike,
                    ..still(attack, strike_at, len(attack), false)
                });
                let back = motions.player(mp::BACK);
                let back_at = strike_at + len(attack);
                out.push(Segment {
                    clip: back,
                    start: back_at,
                    length: len(back).max(0.3),
                    looping: false,
                    from: strike,
                    to: [0.0; 3],
                });
                let settle = motions.player(mp::ATK_TO_NORMAL);
                out.push(still(
                    settle,
                    back_at + len(back).max(0.3),
                    len(settle),
                    false,
                ));
            }
            // Defend: guard for the rest of the command.
            4 => {
                let into = motions.player(mp::IDLE_TO_DEF);
                out.push(still(into, start, len(into), false));
                out.push(still(
                    motions.player(mp::DEFENCE),
                    start + len(into),
                    f32::MAX,
                    true,
                ));
            }
            // Item: one throw around the effect.
            14 => {
                let item = motions.player(mp::ITEM1);
                out.push(still(item, effect - len(item) * 0.5, len(item), false));
            }
            // Magic and skills: chant, then cast at the effect.
            _ => {
                let into = motions.player(mp::IDLE_TO_CHANT);
                let cast = motions.player(mp::MAGIC);
                let cast_at = effect - len(cast) * 0.5;
                out.push(still(into, start, len(into), false));
                out.push(still(
                    motions.player(mp::CHANT),
                    start + len(into),
                    (cast_at - start - len(into)).max(0.0),
                    true,
                ));
                out.push(still(cast, cast_at, len(cast), false));
            }
        },
        Motions::Enemy { attacks, .. } => {
            let (clips, moves) = usize::try_from(running.sub_no)
                .ok()
                .and_then(|sub_no| attacks.get(sub_no))
                .cloned()
                .unwrap_or_default();
            let strike = if moves { strike } else { [0.0; 3] };
            // The clips back to back, the last but one (the blow) ending
            // at the effect when there is room.
            let lengths: Vec<f32> = clips.iter().map(|clip| len(Some(*clip))).collect();
            let blow = lengths
                .len()
                .saturating_sub(2)
                .min(lengths.len().saturating_sub(1));
            let before: f32 = lengths[..(blow + 1).min(lengths.len())].iter().sum();
            let mut at = (effect - before).max(start);
            if moves {
                out.push(Segment {
                    clip: motions.idle(),
                    start: at - RUN_SECONDS,
                    length: RUN_SECONDS,
                    looping: true,
                    from: [0.0; 3],
                    to: strike,
                });
            }
            for (index, (clip, length)) in clips.iter().zip(&lengths).enumerate() {
                let last = index + 1 == clips.len();
                out.push(Segment {
                    clip: Some(*clip),
                    start: at,
                    length: *length,
                    looping: false,
                    from: strike,
                    to: if last { [0.0; 3] } else { strike },
                });
                at += length;
            }
        }
    }
    out
}

/// Damage numbers up at `seconds`: target, amount (negative heals, 0 a
/// miss) and seconds since it landed.
pub fn damage_numbers(stage: &BattleStage, seconds: f64) -> Vec<(usize, i64, f64)> {
    let last = ((seconds.max(0.0) * BATTLE_TICKS_PER_SECOND) as usize + 1).min(stage.frames.len());
    let first = last.saturating_sub((NUMBER_SECONDS * BATTLE_TICKS_PER_SECOND) as usize + 1);
    let mut out = Vec::new();
    for (index, frame) in stage.frames[first..last].iter().enumerate() {
        let since = seconds - seconds_of(first + index);
        if !(0.0..NUMBER_SECONDS).contains(&since) {
            continue;
        }
        for event in &frame.events {
            // Steal shows its message, not a number.
            let stolen_from = |unit: usize| {
                frame.events.iter().any(
                    |event| matches!(event, StageEvent::Stole { target, .. } if *target == unit),
                )
            };
            if let StageEvent::Effect { target, damage, .. } = event
                && !(*damage == 0 && stolen_from(*target))
            {
                out.push((*target, *damage, since));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_tools::battle_stage::{StageFrame, StageUnit};

    fn stage() -> BattleStage {
        let unit = |model, z| StageUnit {
            model,
            position: [0.0, 0.0, z],
            turn: 0.0,
            max_hp: 100,
            max_mp: 0,
            type_no: None,
            name: None,
        };
        let mut stage = BattleStage {
            units: vec![
                unit(
                    StageModel::Player {
                        character: 0,
                        serial: 0,
                    },
                    -1560.0,
                ),
                unit(
                    StageModel::Enemy {
                        geo: 1,
                        idle: 0,
                        hit: 0,
                        death: 0,
                    },
                    1000.0,
                ),
            ],
            ..BattleStage::default()
        };
        for tick in 0..120u64 {
            let mut events = Vec::new();
            if tick == 15 {
                events.push(StageEvent::Command {
                    unit: 0,
                    cmd_no: 1,
                    sub_no: 176,
                    targets: vec![1],
                });
            }
            if tick == 55 {
                events.push(StageEvent::Effect {
                    caster: 0,
                    target: 1,
                    damage: 30,
                });
            }
            stage.frames.push(StageFrame {
                tick,
                hp: vec![100, if tick >= 55 { 70 } else { 100 }],
                atb: vec![(0, 100), (0, 100)],
                present: vec![true, true],
                events,
                ..StageFrame::default()
            });
        }
        stage
    }

    fn player() -> Motions {
        Motions::Player((0..34).map(|index| Some((1, index as u32))).collect())
    }

    #[test]
    fn an_attack_runs_up_strikes_at_the_effect_and_returns() {
        let stage = stage();
        let enemy = Motions::Enemy {
            idle: Some((2, 0)),
            hit: Some((2, 2)),
            death: None,
            attacks: Vec::new(),
        };
        let length = |_: ClipKey| 0.6;
        let at = |seconds: f64| unit_motion(&stage, 0, seconds, &player(), &length);
        // Before the command: idle.
        assert_eq!(at(0.5).clip, Some((1, mp::IDLE_NORMAL as u32)));
        // The blow is centered on the effect (tick 55, 3.67 s).
        let blow = at(55.0 / 15.0);
        assert_eq!(blow.clip, Some((1, mp::ATTACK as u32)));
        assert!(blow.offset[2] > 1000.0, "{:?}", blow.offset);
        // Back home after the command.
        let after = at(6.5);
        assert_eq!(after.offset, [0.0; 3]);
        // The target flinches right after the effect.
        let hit = unit_motion(&stage, 1, 55.0 / 15.0 + 0.1, &enemy, &length);
        assert_eq!(hit.clip, Some((2, 2)));
        let numbers = damage_numbers(&stage, 55.0 / 15.0 + 0.1);
        assert_eq!(numbers.len(), 1);
        assert_eq!((numbers[0].0, numbers[0].1), (1, 30));
        assert!(damage_numbers(&stage, 55.0 / 15.0 + 2.0).is_empty());
    }
}
