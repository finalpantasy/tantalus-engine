//! Field movie (FMV) service state machine.
//!
//! Ported from Memoria `Global/fldfmv.cs` (`FF9FieldFMVDispatch`,
//! `ff9fieldFMVService`, `ff9fieldFMVInit`, `ff9fieldFMVShutdown`,
//! `ff9fieldFMVRestoreState`). Event scripts drive it through the `FMV`
//! opcode (`HostEvent::DispatchCinematic`) and poll it through system
//! variables 14 (`GetCinematicFrame`) and 15 (`SyncCinematic`).
//!
//! The movie decoder itself (`MBG`) is outside this module: a movie is a
//! frame counter that advances once per service call while playing, and the
//! movie "finishes" when the counter reaches the movie's frame count, which
//! fires the reference finish callback (`ff9fieldFMVShutdown`). The frame
//! count comes from the caller; a movie with no known count finishes on its
//! first playing frame, as if skipped.
//!
//! Not modeled: the `FieldMap.FF9FieldAttr` fade/presentation tables, the
//! `FF9.attr`/`FF9Field.attr` bits cleared by `ff9fieldFMVRestoreState`
//! (recorded as a restore count instead), the booster HUD, and the
//! `(attr & 2)` hold-on-last-frame branch of status 6, which only applies
//! after the decoder reports completion and therefore never precedes the
//! finish callback here.

use serde::{Deserialize, Serialize};

/// Field map number that starts playback without waiting for dispatch
/// operation 2 (reference `fldMapNo == 100`).
pub const AUTO_PLAY_FIELD: i32 = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldFmv {
    /// Reference `fldfmv.fmvStatus` (static, zero at startup).
    pub status: i32,
    /// Reference `fldfmv.ff9fieldFMVAttr`.
    pub attr: i32,
    pub disc: i32,
    pub number: i32,
    pub depth: i32,
    /// Current decoder frame (`MBG.GetFrame`).
    pub frame: i32,
    /// Reference `ff9fieldFMVLastFrame`.
    pub last_frame: i32,
    /// Frame count of the seeked movie, if the caller knows it.
    pub frame_count: Option<u32>,
    /// Number of `ff9fieldFMVRestoreState` calls (attr bits not modeled).
    pub restores: u32,
}

impl Default for FieldFmv {
    fn default() -> Self {
        Self::new()
    }
}

impl FieldFmv {
    pub fn new() -> Self {
        Self {
            status: 0,
            attr: 0,
            disc: 0,
            number: 0,
            depth: 0,
            frame: 0,
            last_frame: 0,
            frame_count: None,
            restores: 0,
        }
    }

    /// `FF9FieldFMVDispatch(ParmType, Arg1, Arg2)`. `frame_count` resolves
    /// the frame count of `(disc, number)` for operation 0. Returns the
    /// reference return value (non-zero only for operations 3 and 4).
    pub fn dispatch(
        &mut self,
        operation: i32,
        argument: i32,
        auxiliary: i32,
        field_map_no: i32,
        frame_count: impl Fn(i32, i32) -> Option<u32>,
    ) -> i32 {
        match operation & 15 {
            0 => {
                let disc = (argument >> 8) & 255;
                let number = argument & 255;
                self.init(disc, number, auxiliary, frame_count(disc, number));
                0
            }
            1 => {
                self.shutdown(field_map_no);
                0
            }
            2 => {
                self.attr |= 1;
                0
            }
            3 => self.sync(),
            4 => self.frame,
            5 => {
                self.depth = argument;
                0
            }
            _ => 0,
        }
    }

    /// `FF9FieldFMVSync`: 1 when finished (status 8), 0 while ready or
    /// playing (status 5 to 7), 16 otherwise.
    pub fn sync(&self) -> i32 {
        if self.status == 8 {
            1
        } else if self.status >= 5 {
            0
        } else {
            16
        }
    }

    fn init(&mut self, disc: i32, number: i32, depth: i32, frame_count: Option<u32>) {
        self.attr = 0;
        self.disc = disc;
        self.number = number;
        self.depth = depth;
        self.frame = 0;
        self.frame_count = frame_count;
        self.status = 1;
    }

    /// The movie ended or was skipped: `ff9fieldFMVShutdown`, which the decoder's finish
    /// callback runs in the reference.
    pub fn finish(&mut self, field_map_no: i32) {
        self.shutdown(field_map_no);
    }

    fn shutdown(&mut self, field_map_no: i32) {
        self.attr |= 4;
        self.status = 7;
        self.service(field_map_no);
    }

    fn restore_state(&mut self) {
        self.attr &= -512;
        self.restores += 1;
    }

    /// `ff9fieldFMVService`, called once per field frame before the event
    /// engine runs.
    pub fn service(&mut self, field_map_no: i32) {
        if self.status == 1 || self.status == 2 {
            self.attr |= 8;
            self.status = 3;
        }
        if (self.status == 3 && (self.attr & 8) != 0) || self.status == 4 {
            self.status = 5;
        }
        match self.status {
            5 => {
                if (self.attr & 1) != 0 || field_map_no == AUTO_PLAY_FIELD {
                    // mbg.Play(): playback starts at frame 0.
                    self.frame = 0;
                    self.status = 6;
                }
            }
            6 => {
                self.last_frame = self.frame;
                let finished = match self.frame_count {
                    Some(count) => self.frame as i64 >= i64::from(count),
                    None => true,
                };
                if finished {
                    // MBG finish callback.
                    self.shutdown(field_map_no);
                } else {
                    self.frame += 1;
                }
            }
            7 if (self.attr & 4) != 0 => {
                self.restore_state();
                self.status = 8;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unknown(_: i32, _: i32) -> Option<u32> {
        None
    }

    #[test]
    fn idle_service_reports_not_ready() {
        let fmv = FieldFmv::new();
        assert_eq!(fmv.sync(), 16);
    }

    #[test]
    fn init_decodes_disc_and_number_and_becomes_ready_after_one_service() {
        let mut fmv = FieldFmv::new();
        assert_eq!(
            fmv.dispatch(0, 259, 1, 70, |d, n| Some((d * 10 + n) as u32)),
            0
        );
        assert_eq!((fmv.disc, fmv.number, fmv.depth), (1, 3, 1));
        assert_eq!(fmv.frame_count, Some(13));
        assert_eq!(fmv.status, 1);
        assert_eq!(fmv.sync(), 16);
        fmv.service(70);
        // 1 -> 3 (attr |= 8) -> 5 in the same service call.
        assert_eq!(fmv.status, 5);
        assert_eq!(fmv.sync(), 0);
    }

    #[test]
    fn ready_movie_waits_for_play_request_except_on_field_100() {
        let mut fmv = FieldFmv::new();
        fmv.dispatch(0, 1, 0, 70, unknown);
        fmv.service(70);
        fmv.service(70);
        assert_eq!(fmv.status, 5);

        let mut auto = FieldFmv::new();
        auto.dispatch(0, 1, 0, AUTO_PLAY_FIELD, unknown);
        auto.service(AUTO_PLAY_FIELD);
        assert_eq!(auto.status, 6);
    }

    #[test]
    fn playing_movie_counts_frames_then_finishes_through_shutdown() {
        let mut fmv = FieldFmv::new();
        fmv.dispatch(0, 259, 0, 70, |_, _| Some(3));
        fmv.service(70);
        fmv.dispatch(2, 0, 0, 70, unknown);
        fmv.service(70); // play
        assert_eq!(fmv.status, 6);
        for expected in 1..=3 {
            fmv.service(70);
            assert_eq!(fmv.dispatch(4, 0, 0, 70, unknown), expected);
            assert_eq!(fmv.sync(), 0);
        }
        fmv.service(70); // frame 3 >= 3: finish callback, restore, status 8
        assert_eq!(fmv.status, 8);
        assert_eq!(fmv.sync(), 1);
        assert_eq!(fmv.restores, 1);
        assert_eq!(fmv.attr & !511, 0);
    }

    #[test]
    fn unknown_length_movie_finishes_on_first_playing_frame() {
        let mut fmv = FieldFmv::new();
        fmv.dispatch(0, 259, 0, 70, unknown);
        fmv.service(70);
        fmv.dispatch(2, 0, 0, 70, unknown);
        fmv.service(70);
        fmv.service(70);
        assert_eq!(fmv.sync(), 1);
    }

    #[test]
    fn explicit_shutdown_finishes_immediately() {
        let mut fmv = FieldFmv::new();
        fmv.dispatch(0, 259, 0, 70, |_, _| Some(1000));
        fmv.service(70);
        fmv.dispatch(2, 0, 0, 70, unknown);
        fmv.service(70);
        fmv.dispatch(1, 0, 0, 70, unknown);
        assert_eq!(fmv.status, 8);
        assert_eq!(fmv.sync(), 1);
    }

    #[test]
    fn depth_operation_sets_depth_and_unknown_operations_are_ignored() {
        let mut fmv = FieldFmv::new();
        assert_eq!(fmv.dispatch(5, 1, 0, 70, unknown), 0);
        assert_eq!(fmv.depth, 1);
        assert_eq!(fmv.dispatch(9, 1, 0, 70, unknown), 0);
        // Only the low four bits select the operation.
        assert_eq!(fmv.dispatch(0x13, 0, 0, 70, unknown), 16);
    }
}
