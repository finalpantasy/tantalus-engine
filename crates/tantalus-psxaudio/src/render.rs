//! Offline song renderer: runs the [`Player`] and cuts a clip with loop points.
//!
//! A song that jumps back forever (`0xFE 0x06`) is rendered as intro + two passes of the loop; the
//! clip keeps the intro from the first pass and the loop body from the second pass, so the audio
//! at the loop seam already contains the reverb and release tails of the previous pass and the
//! seam is continuous. A song without a loop is rendered to its end plus the reverb tail.

use crate::akao::{SampleBank, Sequence};
use crate::sequencer::{Player, Stats};
use crate::spu::SAMPLE_RATE;

/// Longest clip the renderer will produce, in seconds (a runaway sequence guard).
pub const MAX_SECONDS: u64 = 12 * 60;
/// Stop a non-looping song once the output stays below this level.
const SILENCE: f32 = 1.0 / 4096.0;

#[derive(Debug, Clone)]
pub struct Rendered {
    /// Interleaved stereo, 44.1 kHz, full scale 1.0.
    pub samples: Vec<f32>,
    /// First and last frame of the loop (inclusive), when the song loops.
    pub loop_frames: Option<(u32, u32)>,
    pub peak: f32,
    /// Linear gain applied to keep the peak under full scale (1.0 when none was needed).
    pub gain: f32,
    pub stats: Stats,
}

impl Rendered {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }
    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(SAMPLE_RATE)
    }
}

/// Renders one song. `bank` is the sample collection the sequence plays on.
pub fn render_song(seq: &Sequence, bank: &SampleBank) -> Rendered {
    render_capped(seq, bank, MAX_SECONDS)
}

/// [`render_song`] with its own length cap in seconds (sound effects use a short one).
pub fn render_capped(seq: &Sequence, bank: &SampleBank, max_seconds: u64) -> Rendered {
    render_looping(seq, bank, max_seconds, max_seconds)
}

/// [`render_capped`], except that a program still playing at `max_seconds` may run on to
/// `looped_max_seconds` for its loop to close (field ambience loops voices of different periods,
/// 12 seconds for program 643 and 40 for program 112, so their loops close late).
pub fn render_looping(
    seq: &Sequence,
    bank: &SampleBank,
    max_seconds: u64,
    looped_max_seconds: u64,
) -> Rendered {
    render_looping_in(seq, bank, max_seconds, looped_max_seconds, None)
}

/// The SPU reverb a sound plays under when its own sequence does not choose one: the console's
/// reverb unit is shared, so a sound effect's reverb sends feed whatever mode and depth the song
/// playing at the time set (every FF9 song header selects mode 4, Studio Large; the depth is the
/// song's own `0xFE 0x02`/`0x03` command).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ambience {
    /// Index into the SDK reverb presets ([`crate::tables::REVERB_PRESETS`]).
    pub preset: u16,
    /// Reverb return volume, 0..=0x7FFF.
    pub depth: i32,
}

/// [`render_looping`] under `ambience` (`None` keeps the sequence's own reverb setting).
pub fn render_looping_in(
    seq: &Sequence,
    bank: &SampleBank,
    max_seconds: u64,
    looped_max_seconds: u64,
    ambience: Option<Ambience>,
) -> Rendered {
    let mut player = Player::new(seq, bank);
    if let Some(a) = ambience {
        player.spu.set_reverb_preset(usize::from(a.preset));
        player.spu.set_reverb_depth(a.depth);
    }
    let mut out: Vec<f32> = Vec::new();
    let chunk = (SAMPLE_RATE / 20) as usize;
    let max_frames = max_seconds * u64::from(SAMPLE_RATE);
    let looped_max_frames = looped_max_seconds.max(max_seconds) * u64::from(SAMPLE_RATE);
    let mut loop_plan: Option<(u64, u64)> = None;
    let mut quiet_frames = 0usize;
    loop {
        let before = out.len();
        player.render(chunk, &mut out);
        if let Some(l) = player.loop_ticks()
            && player.loops_closed()
            && l.length > 0
            && player.tick > l.start + 2 * l.length
        {
            // One more second past the second pass, to align the seam against.
            let end = player
                .tick_frames
                .get((l.start + 2 * l.length) as usize)
                .copied()
                .unwrap_or(player.frames);
            if player.frames >= end + u64::from(SAMPLE_RATE) {
                loop_plan = Some((l.start, l.length));
                break;
            }
        }
        if player.frames >= max_frames && (player.frames >= looped_max_frames || player.all_done())
        {
            break;
        }
        if player.all_done() {
            let level = out[before..].iter().fold(0f32, |m, s| m.max(s.abs()));
            if level < SILENCE && !player.spu.any_active() {
                quiet_frames += chunk;
            } else {
                quiet_frames = 0;
            }
            if quiet_frames >= chunk * 6 {
                break;
            }
        }
    }
    let stats = player.stats.clone();
    let frame_of = |tick: u64| {
        player
            .tick_frames
            .get(tick as usize)
            .copied()
            .unwrap_or(player.frames) as usize
    };
    let (mut samples, loop_frames) = if let Some((start, length)) = loop_plan {
        let s = frame_of(start);
        let e = frame_of(start + length);
        let e2 = frame_of(start + 2 * length);
        let body = e2 - e;
        // The steady-state music is the same in every pass, but the sequencer's timer and the
        // voices' pitch counters are not aligned to whole frames, so pass 2 and pass 3 differ by
        // a small shift. Slide the cut by up to +-1024 frames to where the last samples of the
        // body best match the last samples of the pass before it (a seamless wrap).
        let window = 48usize;
        let mut best = (f64::MAX, 0i64);
        for d in -1024i64..=1024 {
            let a = e as i64 + d;
            let b = e2 as i64 + d;
            if a < window as i64 || (b as usize) + 1 >= out.len() / 2 {
                continue;
            }
            // Wrap: the frame after the body (pass 3, first frame) must resemble the body's start.
            let mut err = 0f64;
            for k in 0..window {
                for c in 0..2 {
                    let x = f64::from(out[(b as usize + k) * 2 + c]);
                    let y = f64::from(out[(a as usize + k) * 2 + c]);
                    err += (x - y).abs();
                }
            }
            if err < best.0 {
                best = (err, d);
            }
        }
        let d = best.1;
        let a = (e as i64 + d) as usize;
        let b = (e2 as i64 + d) as usize;
        let s2 = (s as i64 + d).max(0) as usize;
        let mut clip = out[..s2 * 2].to_vec();
        clip.extend_from_slice(&out[a * 2..b * 2]);
        // Fade the last 2048 frames of the body into the frames that really precede its start
        // (the end of the previous period), so the wrap from the last frame to the first is a
        // step between two consecutive samples of the original render.
        let fade = 2048usize.min(a / 2).min(body / 2);
        let n = clip.len();
        for k in 0..fade {
            let w = (k + 1) as f32 / fade as f32;
            for c in 0..2 {
                let late = out[(b - fade + k) * 2 + c];
                let early = out[(a - fade + k) * 2 + c];
                clip[n - fade * 2 + k * 2 + c] = late * (1.0 - w) + early * w;
            }
        }
        (clip, Some((s2 as u32, (s2 + body - 1) as u32)))
    } else {
        // Trim the silence at the end of a one-shot.
        let mut end = out.len() / 2;
        while end > 1
            && out[(end - 1) * 2].abs() < SILENCE
            && out[(end - 1) * 2 + 1].abs() < SILENCE
        {
            end -= 1;
        }
        let end = (end + (SAMPLE_RATE / 20) as usize).min(out.len() / 2);
        out.truncate(end * 2);
        (out, None)
    };
    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    let mut gain = 1.0;
    if peak > 0.98 {
        gain = 0.98 / peak;
        for s in &mut samples {
            *s *= gain;
        }
    }
    Rendered {
        samples,
        loop_frames,
        peak: peak * gain,
        gain,
        stats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::akao::Articulation;

    /// A one-voice song: a looping tone on instrument 0, a note, a rest, then a jump back to
    /// the start, with a synthetic 16-byte ADPCM block as the only sample.
    fn tiny_song() -> (Sequence, SampleBank) {
        let mut block = vec![0u8; 16];
        block[0] = 0x00;
        block[1] = 0x03;
        block[2..].fill(0x87);
        let bank = SampleBank {
            id: 1,
            spu_dest: 0x1000,
            start_art: 0,
            extra: Vec::new(),
            arts: vec![Articulation {
                id: 0,
                sample_offset: 0,
                loop_offset: 0,
                fine_tune: 0,
                unity_key: 48,
                adsr1: 0x00FF,
                adsr2: 0x0000,
            }],
            adpcm: block,
        };
        let mut data = vec![0u8; 0x48];
        data[0..4].copy_from_slice(b"AKAO");
        data[0x20] = 1; // one channel
        // Channel stream starts at 0x42 (offset field at 0x40 is relative to itself).
        data[0x40] = 2;
        let code = [
            0xFE,
            0x00,
            0x00,
            0x40, // tempo 0x4000
            0xA1,
            0x00,
            0xA5,
            0x04,
            0xA3,
            0x7F,
            0xA8,
            0x7F,          //
            0x00,          // note C, whole note
            0x0D * 11 + 4, // rest, 16th... (note byte 13*11+4)
            0xFE,
            0x06,
            0xEE,
            0xFF, // jump back by 18 bytes to the instrument load
        ];
        data.extend_from_slice(&[0, 0]);
        data.truncate(0x42);
        data.extend_from_slice(&code);
        let len = data.len() as u16;
        data[6..8].copy_from_slice(&len.to_le_bytes());
        // Fix the jump: from the offset field to the first event.
        let jump_at = data.len() as i32 - 2;
        let first = 0x42 + 4_i32;
        let rel = first - jump_at;
        let jump_at_u = jump_at as usize;
        data[jump_at_u..].copy_from_slice(&(rel as i16).to_le_bytes());
        (Sequence::parse(&data, 0).expect("parse"), bank)
    }

    /// The tempo word is added to an accumulator by a 240 Hz timer and a sequencer tick happens
    /// on each 16-bit overflow, so `0x4000` (75 BPM at 48 ticks per quarter note) is 60 ticks per
    /// second. Checked against the discs: renders aligned to the Steam clips at time scale 1.000
    /// (see the sequencer module docs), so the 240 Hz rate is right.
    #[test]
    fn tempo_word_is_ticks_per_second_over_240() {
        let (seq, bank) = tiny_song();
        let mut player = Player::new(&seq, &bank);
        let mut out = Vec::new();
        player.render(SAMPLE_RATE as usize, &mut out);
        assert!((59..=61).contains(&player.tick), "{} ticks", player.tick);
    }

    #[test]
    fn looping_song_renders_with_loop_points_and_no_clipping() {
        let (seq, bank) = tiny_song();
        let r = render_song(&seq, &bank);
        assert!(r.frames() > 10_000, "frames {}", r.frames());
        let (start, end) = r.loop_frames.expect("the song loops");
        assert!(start < end && (end as usize) < r.frames());
        assert!(r.peak > 0.01 && r.peak < 1.0, "peak {}", r.peak);
    }

    /// A sound-effect program that loops the way field ambience does (`0xC8` loop start, a note,
    /// `0xCA` back to it forever) renders with loop points instead of running to the cap.
    #[test]
    fn a_program_looping_forever_gets_loop_points() {
        let (_, bank) = tiny_song();
        let code = vec![
            0xFE,
            0x00,
            0x00,
            0x40, // tempo 0x4000
            0xA1,
            0x00,
            0xA5,
            0x04,
            0xA3,
            0x7F,
            0xA8,
            0x7F, //
            0xC8, // loop start
            0x00, // note
            0x0D * 11 + 4,
            0xCA, // back to the loop start, forever
        ];
        let seq = Sequence {
            id: 0,
            reverb_type: 0,
            sample_set: 0,
            data: code,
            channels: vec![0],
            instrument_map: None,
            drum_map: None,
        };
        let r = render_capped(&seq, &bank, 10);
        let (start, end) = r.loop_frames.expect("the program loops");
        assert!(start < end && (end as usize) < r.frames());
        assert!(r.seconds() < 9.0, "{} s", r.seconds());
    }

    /// A tie straight after a note (the note's gate runs out on the tick the tie is read) holds
    /// the voice; a rest there lets it go.
    #[test]
    fn a_tie_holds_the_note_through_its_gate() {
        let (_, bank) = tiny_song();
        let active_at = |after: u8| {
            let code = vec![
                0xFE, 0x00, 0x00, 0x40, // tempo: 60 ticks per second
                0xA1, 0x00, 0xA5, 0x04, 0xA3, 0x7F, 0xA8, 0x7F, //
                0x02, // note, 48 ticks
                after, 0xA0,
            ];
            let seq = Sequence {
                id: 0,
                reverb_type: 0,
                sample_set: 0,
                data: code,
                channels: vec![0],
                instrument_map: None,
                drum_map: None,
            };
            let mut player = Player::new(&seq, &bank);
            let mut out = Vec::new();
            player.render(SAMPLE_RATE as usize * 6 / 5, &mut out); // 1.2 s = 72 ticks
            player.spu.any_active()
        };
        assert!(active_at(12 * 11 + 2), "tie of 48 ticks");
        assert!(!active_at(13 * 11 + 2), "rest of 48 ticks");
    }

    /// A one-shot with its reverb send on (`0xC2`) rendered under the field ambience carries a
    /// reverb tail past the dry render, and stays the same length and level otherwise.
    #[test]
    fn an_ambience_adds_the_reverb_tail_to_a_send() {
        let (_, bank) = tiny_song();
        let code = vec![
            0xFE, 0x00, 0x00, 0x40, // tempo: 60 ticks per second
            0xA1, 0x00, 0xA5, 0x04, 0xA3, 0x7F, 0xA8, 0x7F, //
            0xC2, // reverb on
            0x02, // note, 48 ticks
            0xA0,
        ];
        let seq = Sequence {
            id: 0,
            reverb_type: 0,
            sample_set: 0,
            data: code,
            channels: vec![0],
            instrument_map: None,
            drum_map: None,
        };
        let dry = render_looping_in(&seq, &bank, 10, 10, None);
        let wet = render_looping_in(
            &seq,
            &bank,
            10,
            10,
            Some(Ambience {
                preset: 4,
                depth: 0x2800,
            }),
        );
        assert_eq!(dry.stats.reverb_notes, 1);
        assert!(
            wet.frames() > dry.frames() + SAMPLE_RATE as usize / 10,
            "dry {} wet {}",
            dry.frames(),
            wet.frames()
        );
    }
}
