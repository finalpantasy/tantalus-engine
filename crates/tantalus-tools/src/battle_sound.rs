//! Sound effects of a live battle, from the data the reference plays them from.
//!
//! * An enemy's attack sequence (`SeqOp::Sfx`, `btlseq.SeqExecSfx`) names the effect and the
//!   frame it plays on: `btl_util.SetBattleSfx(regist, SfxNum, SfxVol)`.
//! * A dying enemy plays effects 121 and 120 as its fade starts (`btl_mot.cs:318`).
//!
//! Party attacks and spells play their sounds from sequence data this engine does not load yet
//! (the party's sequences, and the effect plugin's own sound data), so they are silent here.

use tantalus_assets::pc::battle_sequence::{InterpretOptions, MarkerKind, SequenceFile, interpret};

use crate::battle_stage::{BattleStage, StageEvent};
use crate::play_audio::AudioCue;

/// A sequence sound: the sequence frame it plays on (one battle tick each), effect id, volume.
pub type SequenceSfx = (u32, u16, u8);

/// `btl_mot.cs:318`: the effects an enemy's death fade starts with, in play order.
const ENEMY_DEATH_EFFECTS: [u16; 2] = [121, 120];
const FULL_VOLUME: u8 = 127;

/// The sounds of each sequence of a scene's `.raw17`, by sequence number.
pub fn sequence_sounds(bytes: &[u8]) -> Vec<Vec<SequenceSfx>> {
    let Ok(file) = SequenceFile::parse(bytes, "raw17") else {
        return Vec::new();
    };
    (0..file.slots.len())
        .map(|index| {
            let Some(sequence) = file.sequence(index) else {
                return Vec::new();
            };
            let options = InterpretOptions {
                base_anim: file.slots[index].base_anim,
                ..InterpretOptions::default()
            };
            interpret(sequence, &options)
                .markers
                .into_iter()
                .filter_map(|marker| match marker.kind {
                    MarkerKind::SfxTrigger { id, volume } => Some((marker.frame, id, volume)),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

/// Follows a battle's stage frames and hands out the effects that fall due.
#[derive(Debug, Default)]
pub struct BattleSounds {
    seen: usize,
    tick: u64,
    due: Vec<(u64, u16, u8)>,
}

impl BattleSounds {
    /// Reads the stage frames added since the last call and returns the effects due now.
    pub fn step(&mut self, stage: &BattleStage, party_len: usize) -> Vec<AudioCue> {
        if stage.frames.len() < self.seen {
            *self = Self::default();
        }
        for frame in &stage.frames[self.seen..] {
            for event in &frame.events {
                self.schedule(event, stage, party_len);
            }
        }
        self.seen = stage.frames.len();
        self.tick += 1;
        let tick = self.tick;
        let mut cues = Vec::new();
        self.due.retain(|&(at, id, volume)| {
            let ready = at <= tick;
            if ready {
                cues.push(AudioCue::BattleEffect {
                    id: i32::from(id),
                    volume: i32::from(volume),
                });
            }
            !ready
        });
        cues
    }

    fn schedule(&mut self, event: &StageEvent, stage: &BattleStage, party_len: usize) {
        match *event {
            StageEvent::Command { unit, sub_no, .. } if unit >= party_len => {
                let sounds = usize::try_from(sub_no)
                    .ok()
                    .and_then(|sub| stage.sfx.get(sub));
                for &(frame, id, volume) in sounds.into_iter().flatten() {
                    self.due.push((self.tick + u64::from(frame), id, volume));
                }
            }
            StageEvent::Died { unit } if unit >= party_len => {
                for id in ENEMY_DEATH_EFFECTS {
                    self.due.push((self.tick, id, FULL_VOLUME));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_fall_due_after_their_frame() {
        let mut sounds = BattleSounds::default();
        sounds.due.push((2, 77, 100));
        let stage = BattleStage::default();
        assert!(sounds.step(&stage, 1).is_empty());
        let cues = sounds.step(&stage, 1);
        assert_eq!(cues.len(), 1);
    }
}
