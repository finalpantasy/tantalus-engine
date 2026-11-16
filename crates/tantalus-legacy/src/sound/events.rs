//! Mapping from typed event-VM sound events to sound state transitions.

use tantalus_contracts::EntityId;

use super::dispatch::{SoundSystem, code};
use crate::event::HostEvent;

/// Camera-dependent lookups the field sound opcodes need. `FF9FieldSoundGetPositionVolume`
/// (`FF9Snd.cs:1729`) projects through the field camera and GTE, which this crate does not model,
/// so the host supplies it. Each call returns `(pos, vol)`.
pub trait SoundProjector {
    /// Projects a world position given in the order the opcode read it.
    fn project_position(&mut self, position: [i32; 3]) -> (i32, i32);
    /// Projects an object's current position, or `None` if the object is unknown.
    fn project_object(&mut self, object: EntityId) -> Option<(i32, i32)>;
}

/// What `apply_host_event` did with an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventSoundOutcome {
    /// The event is not a sound event.
    NotSound,
    /// A sound code ran through `FF9Snd.FF9Sound`.
    Dispatched,
    /// `sSEPos` and `sSEVol` were updated.
    PositionSet,
    /// The `btl_flag` continue-field-song bit was updated.
    BattleMusicContinuationSet,
    /// The event needed an object position the projector could not give.
    ObjectUnknown,
}

impl SoundSystem {
    /// `sSEVol = VolPtr * soundVolume >> 7`, clamped to 127. `EventEngine.DoEventCode.cs:1970` to `:1989`.
    pub fn set_se_position_volume(&mut self, pos: i32, vol: i32, script_volume: i32) {
        self.se_pos = pos;
        self.se_vol = ((vol * script_volume) >> 7).min(127);
    }

    /// Applies one host event. Sources: `RunSoundCode*` (opcodes 0xC5 to 0xC8)
    /// `EventEngine.DoEventCode.cs:2474` to `:2519`; `SetSoundPosition` (0x89) `:1970`;
    /// `SetSoundObjectPosition` (0x8A) `:1980`; `SetBattleMusicContinuation` (0x1B) `:269`.
    pub fn apply_host_event(
        &mut self,
        event: &HostEvent,
        projector: &mut dyn SoundProjector,
    ) -> EventSoundOutcome {
        match event {
            HostEvent::RunSound {
                code: sound_code,
                sound,
                arguments: [arg1, arg2, arg3],
            } => {
                // The map 1060 tempo skip (`:2491`) only guards `SONG_TEMPO`, which the dispatcher
                // treats as a no-op, so it has no effect here.
                self.ff9_sound(*sound_code, *sound, *arg1, *arg2, *arg3);
                if self.fld_map_no == 2928 {
                    self.hill_of_despair_sounds(*sound_code, *sound, [*arg1, *arg2, *arg3]);
                }
                EventSoundOutcome::Dispatched
            }
            HostEvent::SetSoundPosition { position, volume } => {
                let (pos, vol) = projector.project_position(*position);
                self.set_se_position_volume(pos, vol, *volume);
                EventSoundOutcome::PositionSet
            }
            HostEvent::SetSoundObjectPosition { object, volume } => {
                match projector.project_object(*object) {
                    Some((pos, vol)) => {
                        self.set_se_position_volume(pos, vol, *volume);
                        EventSoundOutcome::PositionSet
                    }
                    None => EventSoundOutcome::ObjectUnknown,
                }
            }
            HostEvent::SetBattleMusicContinuation { enabled } => {
                self.continue_field_song_after_battle = *enabled;
                EventSoundOutcome::BattleMusicContinuationSet
            }
            _ => EventSoundOutcome::NotSound,
        }
    }

    /// Hill of Despair doubled effects, `EventEngine.DoEventCode.cs:2502` to `:2519`.
    fn hill_of_despair_sounds(&mut self, sound_code: i32, sound: i32, args: [i32; 3]) {
        let [arg1, arg2, arg3] = args;
        if sound == 2786 {
            self.ff9_sound(sound_code, 2980, arg1, arg2, arg3);
            self.ff9_sound(sound_code, 2981, arg1, arg2, arg3);
        } else if sound_code == -12288 && sound == 2787 && arg1 == 0 && arg2 == 128 && arg3 == 125 {
            self.ff9_sound(code::SNDEFFECT_STOP, 2980, 0, 0, 0);
            self.ff9_sound(code::SNDEFFECT_STOP, 2981, 0, 0, 0);
            self.ff9_sound(sound_code, 2982, arg1, arg2, arg3);
            self.ff9_sound(sound_code, 2983, arg1, arg2, arg3);
        } else if sound == 2787 {
            self.ff9_sound(sound_code, 2982, arg1, arg2, arg3);
            self.ff9_sound(sound_code, 2983, arg1, arg2, arg3);
        }
    }
}
