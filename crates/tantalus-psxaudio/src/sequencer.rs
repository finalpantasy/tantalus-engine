//! The AKAO sequence interpreter (FF9, "version 3.2") driving the [`Spu`].
//!
//! Opcode lengths and meanings follow the ff7-flat-wiki "AKAO sequence" page and VGMTrans'
//! `AkaoSeq` reader (see the crate docs). The real driver runs from a 240 Hz timer: each
//! interrupt adds the song tempo (a `u16`) to an accumulator and a sequencer tick happens on
//! each 16-bit overflow, so a tick lasts `65536 / (240 * tempo)` seconds (48 ticks per quarter
//! note).
//!
//! Tempo check (2026-11): the word is a whole number of BPM times 65536/300 in every song (for
//! example 20753 is 95.0 BPM, 19661 is 90.0), which is what the composers' sequencer wrote, so
//! the formula and the 240 Hz rate are right. Onset-envelope alignment of renders against the
//! Steam clips gives a time scale of 1.000 (+-0.001) on every non-looping or long track tried
//! (ids 1, 5, 7, 10, 83, 125, 142, 145). The Steam loop bodies are shorter (median 0.949) because
//! Square cut different loop points, not because the render is slow.
//!
//! Noise and frequency-modulation opcodes never occur in any of the 106 songs (the `Stats`
//! counters are 0 for all of them), so leaving them unmodelled changes no music render.
//!
//! Known approximations, all of them listed in `Stats`: noise and frequency-modulation voices
//! are ignored, LFO shapes are sines, unknown opcodes (`0xE0..0xE2`, `0xFE 0x0B/0x1A..0x1C`)
//! are skipped by length, and volume/pan curves are linear/equal-power rather than the
//! driver's tables.

use std::collections::HashMap;

use crate::akao::{Articulation, SampleBank, Sequence};
use crate::spu::{Adsr, SAMPLE_RATE, Spu, VOICES};

/// Note lengths in ticks for the 11 length slots of a note opcode.
const NOTE_TICKS: [i32; 11] = [192, 96, 48, 24, 12, 6, 3, 32, 16, 8, 4];
/// Timer interrupts per second.
const TIMER_HZ: f64 = 240.0;
/// Events one channel may run in a single tick before it is stopped (guards bad loops).
const EVENT_BUDGET: u32 = 20_000;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Instrument {
    None,
    Art(u32),
    Custom(u8),
}

#[derive(Clone, Copy, Default)]
struct Lfo {
    delay: i32,
    rate: i32,
    depth: i32,
    on: bool,
    age: i32,
}

impl Lfo {
    /// -1..1 sine at the current age, 0 before the delay is over.
    fn value(&self) -> f64 {
        if !self.on || self.age < self.delay {
            return 0.0;
        }
        let period = f64::from(self.rate.max(1));
        ((f64::from(self.age - self.delay) / period) * std::f64::consts::TAU).sin()
    }
}

/// A value that moves linearly to a target over a number of ticks.
#[derive(Clone, Copy, Debug)]
struct Slide {
    value: f64,
    step: f64,
    ticks: i32,
}

impl Slide {
    fn new(value: f64) -> Self {
        Self {
            value,
            step: 0.0,
            ticks: 0,
        }
    }
    fn set(&mut self, value: f64) {
        self.value = value;
        self.ticks = 0;
    }
    fn to(&mut self, target: f64, ticks: i32) {
        let ticks = ticks.max(1);
        self.step = (target - self.value) / f64::from(ticks);
        self.ticks = ticks;
    }
    fn tick(&mut self) {
        if self.ticks > 0 {
            self.value += self.step;
            self.ticks -= 1;
        }
    }
}

struct Channel {
    pc: usize,
    done: bool,
    wait: i32,
    octave: i32,
    transpose: i32,
    instr: Instrument,
    drum: bool,
    master: Slide,
    volume: Slide,
    pan: Slide,
    reverb: bool,
    adsr_override: Option<ChannelAdsr>,
    loop_pos: [usize; 4],
    loop_count: [i32; 4],
    loop_layer: usize,
    pattern_return: Option<usize>,
    one_time_len: Option<i32>,
    fixed_len: i32,
    slur: bool,
    full_note: bool,
    /// The note's gate ran out this tick; the key-off waits until the tick's events have run, so
    /// a tie or a slurred note due on the same tick holds the voice instead.
    release_pending: bool,
    tuning: i32,
    vibrato: Lfo,
    tremolo: Lfo,
    pan_lfo: Lfo,
    pending_bend: Option<(i32, i32)>,
    bend: Slide,
    voice: Option<usize>,
    note_key: i32,
    note_zone_vol: f64,
    note_zone_pan: Option<f64>,
    note_art: Option<Articulation>,
    gate: i32,
    visited: HashMap<usize, u64>,
    /// `(tick of the jump, tick the target was first reached)` for each infinite-loop jump.
    loops: Vec<(u64, u64)>,
    /// The same for each `0xCA` (back to the `0xC8` loop start, forever), the way the sound-effect
    /// programs loop (field ambience such as program 643 on Prima Vista).
    forever: Vec<(u64, u64)>,
}

/// What the song used that the renderer does not model, plus loop facts.
#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub noise_ops: u32,
    pub fm_ops: u32,
    pub unknown_ops: u32,
    pub missing_art: u32,
    pub voice_steals: u32,
    pub budget_stops: u32,
    pub notes: u32,
    /// `(tick, tempo word)` for each tempo command, to compare song lengths.
    pub tempos: Vec<(u64, u16)>,
}

/// Loop description in sequencer ticks, from channels that jump back forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopTicks {
    pub start: u64,
    pub length: u64,
}

pub struct Player<'a> {
    seq: &'a Sequence,
    bank: &'a SampleBank,
    pub spu: Spu,
    chans: Vec<Channel>,
    voice_owner: [Option<usize>; VOICES],
    tempo: f64,
    tick_acc: f64,
    timer_acc: f64,
    pub tick: u64,
    /// Output frame index at the start of each tick.
    pub tick_frames: Vec<u64>,
    pub frames: u64,
    pub stats: Stats,
    condition: u8,
    samples_per_timer: f64,
    /// Remaining ticks and per-tick step of a tempo slide (`0xFE 0x01`).
    tempo_slide: (i32, f64),
}

fn chan_new(pc: usize) -> Channel {
    Channel {
        pc,
        done: false,
        wait: 0,
        octave: 0,
        transpose: 0,
        instr: Instrument::None,
        drum: false,
        master: Slide::new(127.0),
        volume: Slide::new(127.0),
        pan: Slide::new(64.0),
        reverb: false,
        adsr_override: None,
        loop_pos: [0; 4],
        loop_count: [0; 4],
        loop_layer: 0,
        pattern_return: None,
        one_time_len: None,
        fixed_len: 0,
        slur: false,
        full_note: false,
        release_pending: false,
        tuning: 0,
        vibrato: Lfo::default(),
        tremolo: Lfo::default(),
        pan_lfo: Lfo::default(),
        pending_bend: None,
        bend: Slide::new(0.0),
        voice: None,
        note_key: 0,
        note_zone_vol: 1.0,
        note_zone_pan: None,
        note_art: None,
        gate: 0,
        visited: HashMap::new(),
        loops: Vec::new(),
        forever: Vec::new(),
    }
}

impl<'a> Player<'a> {
    pub fn new(seq: &'a Sequence, bank: &'a SampleBank) -> Self {
        let mut spu = Spu::new();
        spu.upload(bank.spu_dest as usize, &bank.adpcm);
        spu.set_reverb_preset(usize::from(seq.reverb_type));
        // Until the song sets a depth the reverb return is at a modest default.
        spu.set_reverb_depth(0x2000);
        let chans = seq.channels.iter().map(|&pc| chan_new(pc)).collect();
        Self {
            seq,
            bank,
            spu,
            chans,
            voice_owner: [None; VOICES],
            tempo: 218.453 * 120.0,
            tick_acc: 0.0,
            timer_acc: 0.0,
            tick: 0,
            tick_frames: Vec::new(),
            frames: 0,
            stats: Stats::default(),
            condition: 0,
            samples_per_timer: f64::from(SAMPLE_RATE) / TIMER_HZ,
            tempo_slide: (0, 0.0),
        }
    }

    pub fn all_done(&self) -> bool {
        self.chans.iter().all(|c| c.done)
    }

    /// Loop facts from the channels that jumped back forever, once each has jumped at least once.
    /// `None` when no channel loops.
    pub fn loop_ticks(&self) -> Option<LoopTicks> {
        let mut start = 0u64;
        let mut length = 0u64;
        let mut any = false;
        for c in &self.chans {
            if let Some(&(jump, arrival)) = self.loops_of(c).first() {
                any = true;
                start = start.max(arrival);
                length = length.max(jump.saturating_sub(arrival));
            }
        }
        any.then_some(LoopTicks { start, length })
    }

    /// True when every channel that is going to loop has done so once.
    pub fn loops_closed(&self) -> bool {
        self.chans
            .iter()
            .all(|c| c.done || !self.loops_of(c).is_empty())
            && self.chans.iter().any(|c| !self.loops_of(c).is_empty())
    }

    /// A channel's loops: its jumps, or when no channel of the song jumps (a sound-effect
    /// program), its `0xCA` loops. Music keeps its jump loops only.
    fn loops_of<'c>(&self, c: &'c Channel) -> &'c [(u64, u64)] {
        if self.chans.iter().any(|c| !c.loops.is_empty()) {
            &c.loops
        } else {
            &c.forever
        }
    }

    /// Renders `frames` stereo frames (interleaved `f32`, full scale 1.0).
    pub fn render(&mut self, frames: usize, out: &mut Vec<f32>) {
        for _ in 0..frames {
            self.timer_acc += 1.0;
            while self.timer_acc >= self.samples_per_timer {
                self.timer_acc -= self.samples_per_timer;
                self.tick_acc += self.tempo;
                while self.tick_acc >= 65536.0 {
                    self.tick_acc -= 65536.0;
                    self.tick_frames.push(self.frames);
                    self.run_tick();
                    self.tick += 1;
                }
            }
            let (l, r) = self.spu.tick();
            out.push(l as f32 / 32768.0);
            out.push(r as f32 / 32768.0);
            self.frames += 1;
        }
    }

    fn u8_at(&self, at: usize) -> u8 {
        self.seq.data.get(at).copied().unwrap_or(0xA0)
    }

    fn s16_at(&self, at: usize) -> i32 {
        let lo = self.u8_at(at);
        let hi = self.u8_at(at + 1);
        i32::from(i16::from_le_bytes([lo, hi]))
    }

    fn run_tick(&mut self) {
        if self.tempo_slide.0 > 0 {
            self.tempo += self.tempo_slide.1;
            self.tempo_slide.0 -= 1;
        }
        for ci in 0..self.chans.len() {
            if self.chans[ci].done {
                continue;
            }
            // Slides and LFOs advance every tick.
            let mut release = false;
            {
                let c = &mut self.chans[ci];
                c.master.tick();
                c.volume.tick();
                c.pan.tick();
                c.bend.tick();
                for lfo in [&mut c.vibrato, &mut c.tremolo, &mut c.pan_lfo] {
                    lfo.age += 1;
                }
                if c.gate > 0 {
                    c.gate -= 1;
                    release = c.gate == 0 && !c.full_note;
                }
                if c.wait > 0 {
                    c.wait -= 1;
                }
            }
            self.chans[ci].release_pending = release;
            let mut budget = EVENT_BUDGET;
            while !self.chans[ci].done && self.chans[ci].wait == 0 {
                if budget == 0 {
                    self.stats.budget_stops += 1;
                    self.chans[ci].done = true;
                    break;
                }
                budget -= 1;
                self.step_event(ci);
            }
            if std::mem::take(&mut self.chans[ci].release_pending) {
                self.key_off_channel(ci);
            }
            self.update_voice(ci);
        }
        // Forget channel ownership of voices that have gone quiet.
        for v in 0..VOICES {
            if let Some(ci) = self.voice_owner[v]
                && !self.spu.is_active(v)
            {
                self.voice_owner[v] = None;
                if self.chans[ci].voice == Some(v) {
                    self.chans[ci].voice = None;
                }
            }
        }
    }

    fn key_off_channel(&mut self, ci: usize) {
        if let Some(v) = self.chans[ci].voice {
            self.spu.key_off(v);
        }
    }

    fn pitch_for(&self, c: &Channel, art: &Articulation, key: i32, unity: i32) -> u32 {
        let semis = f64::from(key - unity)
            + c.bend.value
            + c.vibrato.value() * vibrato_range(c.vibrato.depth);
        let tuning = 1.0 + f64::from(c.tuning) / 128.0;
        let p = 4096.0 * art.tune_multiplier() * 2f64.powf(semis / 12.0) * tuning;
        p.round().clamp(1.0, 16383.0) as u32
    }

    fn volumes(&self, c: &Channel) -> (i32, i32) {
        let mut gain = (c.master.value / 127.0).clamp(0.0, 1.0)
            * (c.volume.value / 127.0).clamp(0.0, 1.0)
            * c.note_zone_vol;
        if c.tremolo.on {
            let d = f64::from(c.tremolo.depth.min(127)) / 127.0;
            gain *= 1.0 - d * (1.0 - c.tremolo.value()) * 0.5;
        }
        let mut pan = c.note_zone_pan.unwrap_or(c.pan.value);
        if c.pan_lfo.on {
            pan += c.pan_lfo.value() * f64::from(c.pan_lfo.depth.min(127)) * 0.5;
        }
        let angle = (pan / 127.0).clamp(0.0, 1.0) * std::f64::consts::FRAC_PI_2;
        let full = 16383.0;
        (
            (gain * angle.cos() * full) as i32,
            (gain * angle.sin() * full) as i32,
        )
    }

    fn update_voice(&mut self, ci: usize) {
        let c = &self.chans[ci];
        let Some(v) = c.voice else { return };
        if self.voice_owner[v] != Some(ci) || !self.spu.is_active(v) {
            return;
        }
        let (l, r) = self.volumes(c);
        let key = c.note_key;
        if let Some(art) = c.note_art {
            let unity = i32::from(art.unity_key);
            let pitch = self.pitch_for(c, &art, key, unity);
            self.spu.set_pitch(v, pitch);
        }
        self.spu.set_volume(v, l, r);
    }

    fn alloc_voice(&mut self, ci: usize) -> usize {
        if let Some(v) = (0..VOICES).find(|&v| !self.spu.is_active(v)) {
            return v;
        }
        // Steal a releasing voice first, then the oldest.
        self.stats.voice_steals += 1;
        let pick = (0..VOICES)
            .filter(|&v| self.spu.is_releasing(v))
            .min_by_key(|&v| self.spu.started_at(v))
            .or_else(|| (0..VOICES).min_by_key(|&v| self.spu.started_at(v)))
            .unwrap_or(0);
        if let Some(owner) = self.voice_owner[pick]
            && owner != ci
            && self.chans[owner].voice == Some(pick)
        {
            self.chans[owner].voice = None;
        }
        pick
    }

    fn resolve(&self, ci: usize, key: i32) -> Option<(Articulation, i32, Zone)> {
        let c = &self.chans[ci];
        let mut zone = Zone::default();
        let (art_id, unity_shift) = if c.drum {
            let map = self.seq.drum_map?;
            let at = map + (key.clamp(0, 127) as usize) * 8;
            let e = self.seq.data.get(at..at + 8)?;
            if e.iter().all(|&b| b == 0) {
                return None;
            }
            zone.adsr = Some((Some(e[2]), Some(e[3]), Some(e[4]), Some(e[5])));
            zone.vol = if e[6] == 0 {
                1.0
            } else {
                f64::from(e[6]) / 128.0
            };
            zone.pan = Some(f64::from(e[7] & 0x7F));
            zone.reverb = Some(e[7] & 0x80 != 0);
            (u32::from(e[0]), key - i32::from(e[1]))
        } else {
            match c.instr {
                Instrument::None => return None,
                Instrument::Art(id) => (id, 0),
                Instrument::Custom(n) => {
                    let table = self.seq.instrument_map? + usize::from(n.min(15)) * 2;
                    let rel = self.s16_at(table);
                    // The pointer table has 16 entries of u16 (0x20 bytes); zones follow it.
                    let mut at = self.seq.instrument_map? + 0x20 + rel.max(0) as usize;
                    let mut found = None;
                    let mut first = None;
                    for _ in 0..64 {
                        let z = self.seq.data.get(at..at + 8)?;
                        if z.iter().all(|&b| b == 0) || z[5] == 0 {
                            break;
                        }
                        first.get_or_insert(z);
                        if key >= i32::from(z[1]) && key <= i32::from(z[2]) {
                            found = Some(z);
                            break;
                        }
                        at += 8;
                    }
                    let z = found.or(first)?;
                    zone.adsr = Some((Some(z[3]), Some(z[4]), Some(z[5]), Some(z[6])));
                    zone.vol = if z[7] == 0 {
                        1.0
                    } else {
                        f64::from(z[7]) / 128.0
                    };
                    (u32::from(z[0]), 0)
                }
            }
        };
        let art = *self.bank.art(art_id)?;
        Some((art, unity_shift, zone))
    }

    fn adsr_for(art: &Articulation, zone: &Zone, over: Option<ChannelAdsr>) -> Adsr {
        let mut a1 = art.adsr1;
        let mut a2 = art.adsr2;
        // Region overrides: attack rate, sustain rate/mode, release rate (VGMTrans' reading of
        // the driver; the decay rate and sustain level always come from the articulation).
        if let Some((ar, sr, sm, rr)) = zone.adsr {
            if let Some(ar) = ar {
                a1 = (a1 & !0x7F00) | (u16::from(ar & 0x7F) << 8);
            }
            a2 &= 0x0020;
            a2 |= u16::from(sr.unwrap_or(0) & 0x7F) << 6;
            let mode = u16::from(sm.unwrap_or(0) & 7);
            a2 |= (mode & 2) << 13 | (mode & 4) << 13;
            a2 |= u16::from(rr.unwrap_or(0) & 0x1F);
        }
        // Channel-level ADSR opcodes (0xAD..0xB7, 0xBB, 0xBF) edit single fields.
        if let Some((ar, dr, sl, sr, rr, modes)) = over {
            if let Some(ar) = ar {
                a1 = (a1 & !0x7F00) | (u16::from(ar & 0x7F) << 8);
            }
            if let Some(dr) = dr {
                a1 = (a1 & !0x00F0) | (u16::from(dr & 0xF) << 4);
            }
            if let Some(sl) = sl {
                a1 = (a1 & !0x000F) | u16::from(sl & 0xF);
            }
            if let Some(sr) = sr {
                a2 = (a2 & !0x1FC0) | (u16::from(sr & 0x7F) << 6);
            }
            if let Some(rr) = rr {
                a2 = (a2 & !0x001F) | u16::from(rr & 0x1F);
            }
            let _ = modes;
        }
        Adsr {
            adsr1: a1,
            adsr2: a2,
        }
    }

    fn note_on(&mut self, ci: usize, rel_key: i32, delta: i32) {
        self.stats.notes += 1;
        // A slurred note takes the voice over; any other note keys the old one off itself.
        self.chans[ci].release_pending = false;
        let c = &self.chans[ci];
        let key = if c.drum {
            c.octave * 12 + rel_key
        } else {
            c.octave * 12 + rel_key + c.transpose
        };
        // Slur: keep the sounding voice and only change its pitch.
        if c.slur
            && let Some(v) = c.voice
            && self.voice_owner[v] == Some(ci)
            && self.spu.is_active(v)
        {
            let c = &mut self.chans[ci];
            c.note_key = key;
            c.gate = delta;
            return;
        }
        // A previous note on this channel is released first unless it was a held (full-length) note.
        self.key_off_channel(ci);
        let Some((art, shift, zone)) = self.resolve(ci, key) else {
            self.stats.missing_art += 1;
            self.chans[ci].gate = delta;
            return;
        };
        let voice = self.alloc_voice(ci);
        {
            let c = &mut self.chans[ci];
            c.note_art = Some(art);
            c.note_zone_vol = zone.vol_or_one();
            c.note_zone_pan = zone.pan;
            c.note_key = key - shift;
            if let Some((len, semis)) = c.pending_bend.take() {
                c.bend.set(f64::from(semis));
                c.bend.to(0.0, len);
                // The note starts away from the written pitch and slides onto it.
            } else if c.bend.ticks == 0 {
                c.bend.set(0.0);
            }
            c.voice = Some(voice);
            c.gate = delta;
            c.vibrato.age = 0;
            c.tremolo.age = 0;
            c.pan_lfo.age = 0;
        }
        let c = &self.chans[ci];
        let adsr = Self::adsr_for(&art, &zone, c.adsr_override);
        let reverb = zone.reverb.unwrap_or(c.reverb);
        let pitch = self.pitch_for(c, &art, c.note_key, i32::from(art.unity_key));
        let (l, r) = self.volumes(c);
        let base = self.bank.spu_dest as usize;
        let addr = base + art.sample_offset as usize;
        let loop_addr = base + art.loop_offset as usize;
        self.spu
            .key_on(voice, addr, loop_addr, pitch, adsr, (l, r), reverb);
        self.voice_owner[voice] = Some(ci);
    }

    fn step_event(&mut self, ci: usize) {
        let tick = self.tick;
        let pc = self.chans[ci].pc;
        self.chans[ci].visited.entry(pc).or_insert(tick);
        let op = self.u8_at(pc);
        let mut next = pc + 1;
        let rd = |s: &Self, i: usize| s.u8_at(pc + i);
        match op {
            0x00..=0x99 | 0xF0..=0xFD => {
                let (note_byte, explicit) = if op >= 0xF0 {
                    (i32::from(op - 0xF0) * 11, Some(i32::from(rd(self, 1))))
                } else {
                    (i32::from(op), None)
                };
                if explicit.is_some() {
                    next += 1;
                }
                let key = note_byte / 11;
                let c = &mut self.chans[ci];
                let mut delta = explicit.unwrap_or(0);
                if let Some(one) = c.one_time_len.take() {
                    delta = one;
                }
                if c.fixed_len != 0 {
                    delta = c.fixed_len;
                }
                if delta == 0 {
                    delta = NOTE_TICKS[(note_byte % 11) as usize];
                }
                match key {
                    0..=11 => self.note_on(ci, key, delta),
                    12 => {
                        // Tie: hold the current note for longer (also when its gate ran out on
                        // this very tick, as a full-length note before a tie does).
                        let c = &mut self.chans[ci];
                        if c.gate > 0 || c.release_pending {
                            c.gate += delta;
                            c.release_pending = false;
                        }
                    }
                    _ => {}
                }
                self.chans[ci].wait = delta;
            }
            0x9A..=0x9F | 0xE3 | 0xE7..=0xEF | 0xFF => {
                // Unimplemented in the driver: it treats them as "finish channel".
                self.stats.unknown_ops += 1;
                self.chans[ci].done = true;
            }
            0xA0 => {
                self.chans[ci].done = true;
            }
            0xA1 => {
                self.chans[ci].instr = Instrument::Art(u32::from(rd(self, 1)));
                self.chans[ci].drum = false;
                next += 1;
            }
            0xA2 => {
                self.chans[ci].one_time_len = Some(i32::from(rd(self, 1)));
                next += 1;
            }
            0xA3 => {
                let v = f64::from(rd(self, 1) & 0x7F);
                self.chans[ci].master.set(v);
                next += 1;
            }
            0xA4 => {
                let len = i32::from(rd(self, 1));
                let semis = i32::from(rd(self, 2) as i8);
                self.chans[ci].pending_bend = Some((if len == 0 { 256 } else { len }, semis));
                next += 2;
            }
            0xA5 => {
                self.chans[ci].octave = i32::from(rd(self, 1));
                next += 1;
            }
            0xA6 => self.chans[ci].octave += 1,
            0xA7 => self.chans[ci].octave -= 1,
            0xA8 => {
                let v = f64::from(rd(self, 1) & 0x7F);
                self.chans[ci].volume.set(v);
                next += 1;
            }
            0xA9 => {
                let len = i32::from(rd(self, 1));
                let v = f64::from(rd(self, 2) & 0x7F);
                self.chans[ci]
                    .volume
                    .to(v, if len == 0 { 256 } else { len });
                next += 2;
            }
            0xAA => {
                let v = f64::from(rd(self, 1) & 0x7F);
                self.chans[ci].pan.set(v);
                next += 1;
            }
            0xAB => {
                let len = i32::from(rd(self, 1));
                let v = f64::from(rd(self, 2) & 0x7F);
                self.chans[ci].pan.to(v, if len == 0 { 256 } else { len });
                next += 2;
            }
            0xAC => {
                self.stats.noise_ops += 1;
                next += 1;
            }
            0xAD..=0xB2 | 0xB7 | 0xBB | 0xBF => {
                let v = rd(self, 1);
                let c = &mut self.chans[ci];
                let mut o = c.adsr_override.unwrap_or_default();
                match op {
                    0xAD => o.0 = Some(v),
                    0xAE => o.1 = Some(v),
                    0xAF => o.2 = Some(v),
                    0xB0 => {
                        o.1 = Some(v);
                        o.2 = Some(rd(self, 2));
                        next += 1;
                    }
                    0xB1 => o.3 = Some(v),
                    0xB2 => o.4 = Some(v),
                    _ => {}
                }
                self.chans[ci].adsr_override = Some(o);
                next += 1;
            }
            0xB3 => self.chans[ci].adsr_override = None,
            0xB4 | 0xB8 => {
                let delay = i32::from(rd(self, 1));
                let rate = i32::from(rd(self, 2));
                let c = &mut self.chans[ci];
                let lfo = if op == 0xB4 {
                    &mut c.vibrato
                } else {
                    &mut c.tremolo
                };
                lfo.delay = delay;
                lfo.rate = if rate == 0 { 256 } else { rate };
                lfo.on = true;
                next += 3;
            }
            0xB5 | 0xB9 | 0xBD => {
                let depth = i32::from(rd(self, 1));
                let c = &mut self.chans[ci];
                match op {
                    0xB5 => c.vibrato.depth = depth,
                    0xB9 => c.tremolo.depth = depth,
                    _ => c.pan_lfo.depth = depth,
                }
                next += 1;
            }
            0xB6 => self.chans[ci].vibrato.on = false,
            0xBA => self.chans[ci].tremolo.on = false,
            0xBC => {
                let rate = i32::from(rd(self, 1));
                let c = &mut self.chans[ci];
                c.pan_lfo.rate = if rate == 0 { 256 } else { rate };
                c.pan_lfo.on = true;
                next += 2;
            }
            0xBE => self.chans[ci].pan_lfo.on = false,
            0xC0 => {
                self.chans[ci].transpose = i32::from(rd(self, 1) as i8);
                next += 1;
            }
            0xC1 => {
                self.chans[ci].transpose += i32::from(rd(self, 1) as i8);
                next += 1;
            }
            0xC2 => self.chans[ci].reverb = true,
            0xC3 => self.chans[ci].reverb = false,
            0xC4 | 0xC5 => self.stats.noise_ops += 1,
            0xC6 | 0xC7 => self.stats.fm_ops += 1,
            0xC8 => {
                let c = &mut self.chans[ci];
                c.loop_layer = (c.loop_layer + 1) & 3;
                c.loop_pos[c.loop_layer] = next;
                c.loop_count[c.loop_layer] = 0;
            }
            0xC9 => {
                let n = i32::from(rd(self, 1));
                let n = if n == 0 { 256 } else { n };
                next += 1;
                let c = &mut self.chans[ci];
                c.loop_count[c.loop_layer] += 1;
                if c.loop_count[c.loop_layer] == n {
                    c.loop_layer = (c.loop_layer + 3) & 3;
                } else {
                    next = c.loop_pos[c.loop_layer];
                }
            }
            0xCA => {
                let tick = self.tick;
                let c = &mut self.chans[ci];
                c.loop_count[c.loop_layer] += 1;
                next = c.loop_pos[c.loop_layer];
                // Only the first pass is used; a loop with no wait in it comes back every op.
                if c.forever.is_empty()
                    && let Some(&arrival) = c.visited.get(&next)
                {
                    c.forever.push((tick, arrival));
                }
            }
            0xCB => {
                let c = &mut self.chans[ci];
                c.reverb = false;
                c.slur = false;
                c.full_note = false;
            }
            0xCC => self.chans[ci].slur = true,
            0xCD => self.chans[ci].slur = false,
            0xCE | 0xCF | 0xD2 | 0xD3 => {
                if op <= 0xCF {
                    self.stats.noise_ops += 1;
                } else {
                    self.stats.fm_ops += 1;
                }
                next += 1;
            }
            0xD0 => self.chans[ci].full_note = true,
            0xD1 => self.chans[ci].full_note = false,
            0xD4..=0xD7 => self.stats.unknown_ops += 1,
            0xD8 => {
                self.chans[ci].tuning = i32::from(rd(self, 1) as i8);
                next += 1;
            }
            0xD9 => {
                self.chans[ci].tuning += i32::from(rd(self, 1) as i8);
                next += 1;
            }
            0xDA => next += 1,
            0xDB => {}
            0xDC => {
                self.chans[ci].fixed_len += i32::from(rd(self, 1) as i8);
                next += 1;
            }
            0xDD..=0xDF => {
                let depth = i32::from(rd(self, 2));
                let c = &mut self.chans[ci];
                match op {
                    0xDD => c.vibrato.depth = depth,
                    0xDE => c.tremolo.depth = depth,
                    _ => c.pan_lfo.depth = depth,
                }
                next += 2;
            }
            0xE0 | 0xE2 => self.stats.unknown_ops += 1,
            0xE1 => {
                self.stats.unknown_ops += 1;
                next += 1;
            }
            0xE4..=0xE6 => {
                let rate = i32::from(rd(self, 2));
                let c = &mut self.chans[ci];
                let lfo = match op {
                    0xE4 => &mut c.vibrato,
                    0xE5 => &mut c.tremolo,
                    _ => &mut c.pan_lfo,
                };
                lfo.rate = if rate == 0 { 256 } else { rate };
                next += 2;
            }
            0xFE => {
                let sub = rd(self, 1);
                next += 1;
                next = self.step_extended(ci, pc, sub, next);
            }
        }
        self.chans[ci].pc = next;
    }

    /// `0xFE <sub>` commands; `next` points past the sub-opcode. Returns the next pc.
    fn step_extended(&mut self, ci: usize, pc: usize, sub: u8, mut next: usize) -> usize {
        let a = |s: &Self, i: usize| s.u8_at(pc + i);
        match sub {
            0x00 => {
                let t = u16::from_le_bytes([a(self, 2), a(self, 3)]);
                self.tempo = f64::from(t.max(1));
                self.stats.tempos.push((self.tick, t));
                next += 2;
            }
            0x01 => {
                let len = i32::from(a(self, 2));
                let len = if len == 0 { 256 } else { len };
                let t = f64::from(u16::from_le_bytes([a(self, 3), a(self, 4)]).max(1));
                self.stats.tempos.push((self.tick, t as u16));
                self.tempo_slide = (len, (t - self.tempo) / f64::from(len));
                next += 3;
            }
            0x02 => {
                let d = i32::from(u16::from_le_bytes([a(self, 2), a(self, 3)]));
                self.spu.set_reverb_depth(d.min(0x7FFF));
                next += 2;
            }
            0x03 => {
                let d = i32::from(u16::from_le_bytes([a(self, 3), a(self, 4)]));
                self.spu.set_reverb_depth(d.min(0x7FFF));
                next += 3;
            }
            0x04 => self.chans[ci].drum = true,
            0x05 => self.chans[ci].drum = false,
            0x06 => {
                let rel = self.s16_at(pc + 2);
                let dest = (pc as i32 + 2 + rel) as usize;
                let tick = self.tick;
                let c = &mut self.chans[ci];
                if dest <= pc
                    && let Some(&arrival) = c.visited.get(&dest)
                {
                    c.loops.push((tick, arrival));
                }
                next = dest;
            }
            0x07 => {
                let cond = a(self, 2);
                let rel = self.s16_at(pc + 3);
                if cond == self.condition {
                    next = (pc as i32 + 3 + rel) as usize;
                } else {
                    next += 3;
                }
            }
            0x08 | 0x09 => {
                let n = i32::from(a(self, 2));
                let n = if n == 0 { 256 } else { n };
                let rel = self.s16_at(pc + 3);
                let dest = (pc as i32 + 3 + rel) as usize;
                let c = &mut self.chans[ci];
                let hit = c.loop_count[c.loop_layer] + 1 == n;
                next += 3;
                if hit {
                    next = dest;
                }
                if sub == 0x09 {
                    c.loop_layer = (c.loop_layer + 3) & 3;
                }
            }
            0x0A => {
                self.chans[ci].instr = Instrument::Art(u32::from(a(self, 2)));
                self.chans[ci].drum = false;
                next += 1;
            }
            0x0B => {
                self.stats.unknown_ops += 1;
                next += 4;
            }
            0x0E => {
                let rel = self.s16_at(pc + 2);
                self.chans[ci].pattern_return = Some(next + 2);
                next = (pc as i32 + 2 + rel) as usize;
            }
            0x0F => {
                if let Some(back) = self.chans[ci].pattern_return.take() {
                    next = back;
                }
            }
            0x10 => next += 1,
            0x11 | 0x1D | 0x1E => {}
            0x12 => {
                let len = i32::from(a(self, 2));
                let v = f64::from(a(self, 3) & 0x7F);
                self.chans[ci]
                    .master
                    .to(v, if len == 0 { 256 } else { len });
                next += 2;
            }
            0x14 => {
                self.chans[ci].instr = Instrument::Custom(a(self, 2));
                self.chans[ci].drum = false;
                next += 1;
            }
            0x15 => next += 2,
            0x16 => next += 1,
            0x19 => {
                self.stats.unknown_ops += 1;
                next += 2;
            }
            0x1A | 0x1B => self.stats.unknown_ops += 1,
            0x1C => {
                next += 1;
            }
            _ => {
                self.stats.unknown_ops += 1;
                self.chans[ci].done = true;
            }
        }
        next
    }
}

/// Vibrato range in semitones for a depth byte (psx notes: MSB 0 is about +-50 cents, MSB 1 is
/// about +-700 cents).
fn vibrato_range(depth: i32) -> f64 {
    let d = f64::from(depth & 0x7F) / 127.0;
    if depth & 0x80 == 0 { 0.5 * d } else { 7.0 * d }
}

type ChannelAdsr = (
    Option<u8>,
    Option<u8>,
    Option<u8>,
    Option<u8>,
    Option<u8>,
    Option<u8>,
);

type ZoneAdsr = (Option<u8>, Option<u8>, Option<u8>, Option<u8>);

#[derive(Default, Clone, Copy)]
struct Zone {
    adsr: Option<ZoneAdsr>,
    vol: f64,
    pan: Option<f64>,
    reverb: Option<bool>,
}

impl Zone {
    fn vol_or_one(&self) -> f64 {
        if self.vol == 0.0 { 1.0 } else { self.vol }
    }
}
