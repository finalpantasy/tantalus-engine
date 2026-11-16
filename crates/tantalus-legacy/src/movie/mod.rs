//! Movie playback timing, ported from Memoria's `MovieMaterial`.
//!
//! This is the clock only: it decides which decode time to show and when to start the audio,
//! loop or finish. It does no decoding, drawing or device I/O. The host feeds it the audio
//! position, the frame delta and the decoder's reported times each tick, and applies the events.

mod clock;

pub use clock::{ClockEvent, ClockInput, ClockOutput, MovieClock};
