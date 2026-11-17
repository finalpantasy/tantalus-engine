//! A model of the PS1 SPU: 24 ADPCM voices with ADSR envelopes, 4-point Gaussian interpolation,
//! per-voice stereo volume and the reverb unit, producing 44.1 kHz stereo.
//!
//! Written from the psx-spx "Sound Processing Unit" document (see the crate docs). Not modelled:
//! noise, pitch modulation, sweep volumes and the CD/external inputs; FF9's songs use none of
//! them in a way that matters for a listening render.

use crate::tables::{GAUSS, REVERB_PRESETS};

pub const VOICES: usize = 24;
pub const RAM_SIZE: usize = 0x80000;
pub const SAMPLE_RATE: u32 = 44_100;

/// SPU-ADPCM prediction filters (`K0`, `K1`), `x / 64`.
const FILTERS: [(i32, i32); 5] = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Off,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// ADSR settings as the two SPU registers hold them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Adsr {
    pub adsr1: u16,
    pub adsr2: u16,
}

#[derive(Clone)]
struct Voice {
    phase: Phase,
    adsr: Adsr,
    level: i32,
    env_counter: u32,
    addr: usize,
    loop_addr: usize,
    /// Decoded samples: three from the previous block, then the current block's 28.
    ring: [i16; 31],
    old: i32,
    older: i32,
    counter: u32,
    pitch: u32,
    vol_l: i32,
    vol_r: i32,
    reverb: bool,
    /// Last mixed output of the voice (for the oldest-voice steal rule).
    started: u64,
}

impl Voice {
    fn idle() -> Self {
        Self {
            phase: Phase::Off,
            adsr: Adsr::default(),
            level: 0,
            env_counter: 0,
            addr: 0,
            loop_addr: 0,
            ring: [0; 31],
            old: 0,
            older: 0,
            counter: 0,
            pitch: 0,
            vol_l: 0,
            vol_r: 0,
            reverb: false,
            started: 0,
        }
    }
}

/// The reverb unit's registers, from one of the SDK presets.
#[derive(Clone, Copy, Default)]
struct ReverbRegs {
    d_apf1: i32,
    d_apf2: i32,
    v_iir: i32,
    v_comb: [i32; 4],
    v_wall: i32,
    v_apf1: i32,
    v_apf2: i32,
    m_lsame: i32,
    m_rsame: i32,
    m_lcomb: [i32; 4],
    m_rcomb: [i32; 4],
    d_lsame: i32,
    d_rsame: i32,
    m_ldiff: i32,
    m_rdiff: i32,
    d_ldiff: i32,
    d_rdiff: i32,
    m_lapf1: i32,
    m_rapf1: i32,
    m_lapf2: i32,
    m_rapf2: i32,
    v_lin: i32,
    v_rin: i32,
}

/// The FIR that resamples between the 44.1 kHz mixer and the 22.05 kHz reverb unit.
const REVERB_FIR: [i32; 39] = [
    -1, 0, 2, 0, -10, 0, 35, 0, -103, 0, 266, 0, -616, 0, 1332, 0, -2960, 0, 10246, 16384, 10246,
    0, -2960, 0, 1332, 0, -616, 0, 266, 0, -103, 0, 35, 0, -10, 0, 2, 0, -1,
];

pub struct Spu {
    pub ram: Vec<u8>,
    voices: Vec<Voice>,
    clock: u64,
    reverb_regs: ReverbRegs,
    reverb_esa: usize,
    reverb_pos: usize,
    reverb_on: bool,
    reverb_out: (i32, i32),
    /// Reverb output volume (signed 16-bit register scale).
    pub reverb_depth: i32,
    in_hist: Vec<(i32, i32)>,
    out_hist: Vec<(i32, i32)>,
    phase_even: bool,
}

fn clamp16(x: i32) -> i32 {
    x.clamp(-0x8000, 0x7FFF)
}

impl Default for Spu {
    fn default() -> Self {
        Self::new()
    }
}

impl Spu {
    pub fn new() -> Self {
        let mut spu = Self {
            ram: vec![0; RAM_SIZE],
            voices: vec![Voice::idle(); VOICES],
            clock: 0,
            reverb_regs: ReverbRegs::default(),
            reverb_esa: RAM_SIZE - 0x10,
            reverb_pos: 0,
            reverb_on: false,
            reverb_out: (0, 0),
            reverb_depth: 0,
            in_hist: vec![(0, 0); 39],
            out_hist: vec![(0, 0); 39],
            phase_even: false,
        };
        spu.set_reverb_preset(0);
        spu
    }

    /// Copies sample data into SPU RAM.
    pub fn upload(&mut self, at: usize, data: &[u8]) {
        let end = (at + data.len()).min(RAM_SIZE);
        if at < end {
            self.ram[at..end].copy_from_slice(&data[..end - at]);
        }
    }

    /// Loads one of the SDK reverb presets (0 = off) and clears its work area.
    pub fn set_reverb_preset(&mut self, index: usize) {
        let (size, regs) = REVERB_PRESETS[index.min(REVERB_PRESETS.len() - 1)];
        let r = |i: usize| i32::from(regs[i]);
        let s = |i: usize| i32::from(regs[i] as i16);
        self.reverb_regs = ReverbRegs {
            d_apf1: r(0),
            d_apf2: r(1),
            v_iir: s(2),
            v_comb: [s(3), s(4), s(5), s(6)],
            v_wall: s(7),
            v_apf1: s(8),
            v_apf2: s(9),
            m_lsame: r(10),
            m_rsame: r(11),
            m_lcomb: [r(12), r(14), r(20), r(22)],
            m_rcomb: [r(13), r(15), r(21), r(23)],
            d_lsame: r(16),
            d_rsame: r(17),
            m_ldiff: r(18),
            m_rdiff: r(19),
            d_ldiff: r(24),
            d_rdiff: r(25),
            m_lapf1: r(26),
            m_rapf1: r(27),
            m_lapf2: r(28),
            m_rapf2: r(29),
            v_lin: s(30),
            v_rin: s(31),
        };
        self.reverb_on = index != 0;
        self.reverb_esa = RAM_SIZE - size as usize;
        self.reverb_pos = self.reverb_esa;
        self.ram[self.reverb_esa..].fill(0);
    }

    /// Reverb output volume as a fraction of full scale.
    pub fn set_reverb_depth(&mut self, depth: i32) {
        self.reverb_depth = depth.clamp(-0x8000, 0x7FFF);
    }

    /// Starts a voice. `volume` is the left/right gain, 0..=0x7FFF full scale.
    #[allow(clippy::too_many_arguments)]
    pub fn key_on(
        &mut self,
        voice: usize,
        addr: usize,
        loop_addr: usize,
        pitch: u32,
        adsr: Adsr,
        vol: (i32, i32),
        reverb: bool,
    ) {
        self.clock += 1;
        let v = &mut self.voices[voice];
        *v = Voice {
            phase: Phase::Attack,
            adsr,
            addr,
            loop_addr,
            pitch: pitch.min(0x3FFF),
            vol_l: vol.0,
            vol_r: vol.1,
            reverb,
            started: self.clock,
            // Force a block decode on the first sample.
            counter: 28 << 12,
            ..Voice::idle()
        };
        v.phase = Phase::Attack;
    }

    pub fn key_off(&mut self, voice: usize) {
        let v = &mut self.voices[voice];
        if v.phase != Phase::Off {
            v.phase = Phase::Release;
            v.env_counter = 0;
        }
    }

    pub fn set_pitch(&mut self, voice: usize, pitch: u32) {
        self.voices[voice].pitch = pitch.min(0x3FFF);
    }

    pub fn set_volume(&mut self, voice: usize, l: i32, r: i32) {
        let v = &mut self.voices[voice];
        v.vol_l = l;
        v.vol_r = r;
    }

    pub fn set_adsr(&mut self, voice: usize, adsr: Adsr) {
        self.voices[voice].adsr = adsr;
    }

    pub fn set_voice_reverb(&mut self, voice: usize, on: bool) {
        self.voices[voice].reverb = on;
    }

    /// True while the voice makes (or may still make) sound.
    pub fn is_active(&self, voice: usize) -> bool {
        self.voices[voice].phase != Phase::Off
    }

    pub fn is_releasing(&self, voice: usize) -> bool {
        self.voices[voice].phase == Phase::Release
    }

    /// Age rank for stealing: lower is older.
    pub fn started_at(&self, voice: usize) -> u64 {
        self.voices[voice].started
    }

    pub fn any_active(&self) -> bool {
        self.voices.iter().any(|v| v.phase != Phase::Off)
    }

    fn decode_block(&mut self, vi: usize) {
        let ram = &self.ram;
        let v = &mut self.voices[vi];
        if v.addr + 16 > RAM_SIZE {
            v.phase = Phase::Off;
            return;
        }
        let block = &ram[v.addr..v.addr + 16];
        let shift = i32::from(block[0] & 0x0F);
        let filter = usize::from((block[0] >> 4) & 7).min(4);
        let flags = block[1];
        if flags & 4 != 0 {
            v.loop_addr = v.addr;
        }
        // The last three samples of the block that just ended come first in the ring.
        let tail = [v.ring[28], v.ring[29], v.ring[30]];
        v.ring[0..3].copy_from_slice(&tail);
        let (k0, k1) = FILTERS[filter];
        // A shift above 12 behaves like 9 (psx-spx: "illegal" values).
        let shift = if shift > 12 { 9 } else { shift };
        for i in 0..28 {
            let byte = block[2 + i / 2];
            let nib = if i % 2 == 0 { byte & 0x0F } else { byte >> 4 };
            let signed = i32::from(((u16::from(nib)) << 12) as i16) >> shift;
            let predicted = (v.old * k0 + v.older * k1 + 32) >> 6;
            let sample = clamp16(signed + predicted);
            v.older = v.old;
            v.old = sample;
            v.ring[3 + i] = sample as i16;
        }
        if flags & 1 != 0 {
            v.addr = v.loop_addr;
            if flags & 2 == 0 {
                // End without repeat: the voice is muted and released.
                v.phase = Phase::Release;
                v.level = 0;
            }
        } else {
            v.addr += 16;
        }
    }

    fn step_envelope(v: &mut Voice) {
        let (exp, decrease, shift, step, target) = match v.phase {
            Phase::Off => return,
            Phase::Attack => (
                v.adsr.adsr1 & 0x8000 != 0,
                false,
                i32::from((v.adsr.adsr1 >> 10) & 0x1F),
                i32::from((v.adsr.adsr1 >> 8) & 3),
                0x7FFF,
            ),
            Phase::Decay => (
                true,
                true,
                i32::from((v.adsr.adsr1 >> 4) & 0xF),
                0,
                ((i32::from(v.adsr.adsr1 & 0xF)) + 1) * 0x800,
            ),
            Phase::Sustain => (
                v.adsr.adsr2 & 0x8000 != 0,
                v.adsr.adsr2 & 0x4000 != 0,
                i32::from((v.adsr.adsr2 >> 8) & 0x1F),
                i32::from((v.adsr.adsr2 >> 6) & 3),
                0,
            ),
            Phase::Release => (
                v.adsr.adsr2 & 0x20 != 0,
                true,
                i32::from(v.adsr.adsr2 & 0x1F),
                0,
                0,
            ),
        };
        // psx-spx "Envelope Operation depending on Shift/Step/Mode/Direction".
        let mut adsr_step = 7 - step;
        if decrease {
            adsr_step = !adsr_step;
        }
        adsr_step <<= (11 - shift).max(0);
        let mut increment: i32 = 0x8000 >> (shift - 11).max(0);
        if exp && !decrease && v.level > 0x6000 {
            if shift < 10 {
                adsr_step >>= 2;
            } else if shift >= 11 {
                increment >>= 2;
            } else {
                adsr_step >>= 1;
                increment >>= 1;
            }
        } else if exp && decrease {
            adsr_step = adsr_step * v.level / 0x8000;
        }
        let all_ones = (step | (shift << 2))
            == if v.phase == Phase::Release {
                0x7C
            } else {
                0x7F
            };
        if !all_ones {
            increment = increment.max(1);
        }
        v.env_counter += increment as u32;
        if v.env_counter & 0x8000 == 0 {
            // Still evaluate phase changes below.
        } else {
            v.env_counter = 0;
            v.level += adsr_step;
            if decrease {
                v.level = v.level.max(0);
            } else {
                v.level = v.level.clamp(0, 0x7FFF);
            }
        }
        match v.phase {
            Phase::Attack if v.level >= 0x7FFF => {
                v.level = 0x7FFF;
                v.phase = Phase::Decay;
                v.env_counter = 0;
            }
            Phase::Decay if v.level <= target => {
                v.phase = Phase::Sustain;
                v.env_counter = 0;
            }
            Phase::Release if v.level <= 0 => {
                v.phase = Phase::Off;
            }
            _ => {}
        }
    }

    /// One output frame at 44.1 kHz: `(left, right)` in i16 range.
    pub fn tick(&mut self) -> (i32, i32) {
        let mut left = 0i32;
        let mut right = 0i32;
        let mut rev_l = 0i32;
        let mut rev_r = 0i32;
        for vi in 0..VOICES {
            if self.voices[vi].phase == Phase::Off {
                continue;
            }
            // Advance the pitch counter and decode blocks as they are crossed.
            let (sample, level) = {
                if self.voices[vi].counter >= 28 << 12 {
                    self.voices[vi].counter -= 28 << 12;
                    self.decode_block(vi);
                    if self.voices[vi].phase == Phase::Off {
                        continue;
                    }
                }
                let v = &self.voices[vi];
                let idx = (v.counter >> 12) as usize;
                let g = ((v.counter >> 4) & 0xFF) as usize;
                // Ring index of "new" is idx + 3; older samples sit below it.
                let new = i32::from(v.ring[idx + 3]);
                let old = i32::from(v.ring[idx + 2]);
                let older = i32::from(v.ring[idx + 1]);
                let oldest = i32::from(v.ring[idx]);
                let mut out = (i32::from(GAUSS[0xFF - g]) * oldest) >> 15;
                out += (i32::from(GAUSS[0x1FF - g]) * older) >> 15;
                out += (i32::from(GAUSS[0x100 + g]) * old) >> 15;
                out += (i32::from(GAUSS[g]) * new) >> 15;
                (out, v.level)
            };
            let v = &mut self.voices[vi];
            v.counter += v.pitch;
            Self::step_envelope(v);
            let s = (sample * level) >> 15;
            let l = (s * v.vol_l) >> 15;
            let r = (s * v.vol_r) >> 15;
            left += l;
            right += r;
            if v.reverb {
                rev_l += l;
                rev_r += r;
            }
        }
        // Reverb: the unit runs at 22.05 kHz; the FIR resamples in and out.
        self.in_hist.rotate_right(1);
        self.in_hist[0] = (rev_l, rev_r);
        self.out_hist.rotate_right(1);
        self.out_hist[0] = (0, 0);
        self.phase_even = !self.phase_even;
        if self.phase_even {
            let (mut il, mut ir) = (0i32, 0i32);
            for (tap, c) in REVERB_FIR.iter().enumerate() {
                if *c != 0 {
                    il += self.in_hist[tap].0 * c;
                    ir += self.in_hist[tap].1 * c;
                }
            }
            let il = clamp16(il >> 15);
            let ir = clamp16(ir >> 15);
            self.reverb_out = if self.reverb_on {
                self.reverb_step(il, ir)
            } else {
                (0, 0)
            };
            self.out_hist[0] = self.reverb_out;
        }
        // Upsample: zero-stuffed 22.05 kHz stream through the same FIR, gain 2.
        let (mut ol, mut or) = (0i64, 0i64);
        for (tap, c) in REVERB_FIR.iter().enumerate() {
            if *c != 0 {
                ol += i64::from(self.out_hist[tap].0) * i64::from(*c);
                or += i64::from(self.out_hist[tap].1) * i64::from(*c);
            }
        }
        // `out_hist` only holds a value on every other frame, so the tap sum is already
        // delayed by the filter; scale by 2 for the zero stuffing.
        let ol = ((ol * 2) >> 15) as i32;
        let or = ((or * 2) >> 15) as i32;
        left += (ol * self.reverb_depth) >> 15;
        right += (or * self.reverb_depth) >> 15;
        (clamp16(left), clamp16(right))
    }

    fn rv_index(&self, reg: i32, extra: i32) -> usize {
        let size = (RAM_SIZE - self.reverb_esa) as i32;
        let rel = (self.reverb_pos as i32 - self.reverb_esa as i32) + reg * 8 + extra;
        self.reverb_esa + rel.rem_euclid(size) as usize
    }

    fn rv_read(&self, reg: i32, extra: i32) -> i32 {
        let i = self.rv_index(reg, extra) & !1;
        i32::from(i16::from_le_bytes([self.ram[i], self.ram[i + 1]]))
    }

    fn rv_write(&mut self, reg: i32, extra: i32, value: i32) {
        let i = self.rv_index(reg, extra) & !1;
        let b = (clamp16(value) as i16).to_le_bytes();
        self.ram[i] = b[0];
        self.ram[i + 1] = b[1];
    }

    /// One 22.05 kHz reverb cycle (psx-spx "SPU Reverb Formula"); returns the unit's output.
    fn reverb_step(&mut self, in_l: i32, in_r: i32) -> (i32, i32) {
        let r = self.reverb_regs;
        let lin = (in_l * r.v_lin) >> 15;
        let rin = (in_r * r.v_rin) >> 15;
        let m = |a: i32, b: i32| (a * b) >> 15;
        // Same side reflection.
        let l_same_prev = self.rv_read(r.m_lsame, -2);
        let r_same_prev = self.rv_read(r.m_rsame, -2);
        let d_lsame = self.rv_read(r.d_lsame, 0);
        let d_rsame = self.rv_read(r.d_rsame, 0);
        let d_ldiff = self.rv_read(r.d_ldiff, 0);
        let d_rdiff = self.rv_read(r.d_rdiff, 0);
        let l_diff_prev = self.rv_read(r.m_ldiff, -2);
        let r_diff_prev = self.rv_read(r.m_rdiff, -2);
        let w_lsame = m(lin + m(d_lsame, r.v_wall) - l_same_prev, r.v_iir) + l_same_prev;
        let w_rsame = m(rin + m(d_rsame, r.v_wall) - r_same_prev, r.v_iir) + r_same_prev;
        self.rv_write(r.m_lsame, 0, w_lsame);
        self.rv_write(r.m_rsame, 0, w_rsame);
        // Different side reflection.
        let w_ldiff = m(lin + m(d_rdiff, r.v_wall) - l_diff_prev, r.v_iir) + l_diff_prev;
        let w_rdiff = m(rin + m(d_ldiff, r.v_wall) - r_diff_prev, r.v_iir) + r_diff_prev;
        self.rv_write(r.m_ldiff, 0, w_ldiff);
        self.rv_write(r.m_rdiff, 0, w_rdiff);
        // Early echo.
        let mut lout = 0;
        let mut rout = 0;
        for i in 0..4 {
            lout += m(r.v_comb[i], self.rv_read(r.m_lcomb[i], 0));
            rout += m(r.v_comb[i], self.rv_read(r.m_rcomb[i], 0));
        }
        // Late reverb: two all-pass filters.
        let apf = |spu: &mut Self, mem: i32, delay: i32, vol: i32, x: i32| {
            let back = spu.rv_read(mem - delay, 0);
            let y = clamp16(x - m(vol, back));
            spu.rv_write(mem, 0, y);
            clamp16(m(y, vol) + back)
        };
        lout = apf(self, r.m_lapf1, r.d_apf1, r.v_apf1, lout);
        rout = apf(self, r.m_rapf1, r.d_apf1, r.v_apf1, rout);
        lout = apf(self, r.m_lapf2, r.d_apf2, r.v_apf2, lout);
        rout = apf(self, r.m_rapf2, r.d_apf2, r.v_apf2, rout);
        // Advance the buffer address.
        self.reverb_pos += 2;
        if self.reverb_pos >= RAM_SIZE {
            self.reverb_pos = self.reverb_esa;
        }
        (clamp16(lout), clamp16(rout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One ADPCM block of a constant tone at the loudest shift with filter 0, looping onto itself.
    fn looping_block() -> Vec<u8> {
        let mut block = vec![0u8; 16];
        block[0] = 0x00; // shift 0 (loudest), filter 0
        block[1] = 0x03; // loop end + repeat
        for i in 0..14 {
            // Alternating +7 / -8 nibbles: a square wave at the sample rate / 2.
            block[2 + i] = 0x87;
        }
        block
    }

    #[test]
    fn voice_plays_loops_and_releases() {
        let mut spu = Spu::new();
        spu.upload(0x1000, &looping_block());
        // Fast attack, sustain at max, immediate release.
        let adsr = Adsr {
            adsr1: 0x00FF,
            adsr2: 0x0000,
        };
        spu.key_on(0, 0x1000, 0x1000, 0x1000, adsr, (0x3FFF, 0x3FFF), false);
        let mut peak = 0;
        for _ in 0..2000 {
            let (l, r) = spu.tick();
            assert_eq!(l, r);
            peak = peak.max(l.abs());
        }
        assert!(peak > 1000, "voice should be audible, peak {peak}");
        assert!(spu.is_active(0));
        spu.key_off(0);
        for _ in 0..200_000 {
            spu.tick();
        }
        assert!(!spu.is_active(0), "voice should fall silent after release");
    }

    #[test]
    fn adpcm_end_without_repeat_mutes() {
        let mut spu = Spu::new();
        let mut block = looping_block();
        block[1] = 0x01; // end, no repeat
        spu.upload(0x1000, &block);
        spu.key_on(
            1,
            0x1000,
            0x1000,
            0x1000,
            Adsr {
                adsr1: 0x00FF,
                adsr2: 0x0000,
            },
            (0x3FFF, 0x3FFF),
            false,
        );
        for _ in 0..500_000 {
            spu.tick();
        }
        assert!(!spu.is_active(1));
    }

    #[test]
    fn reverb_adds_a_tail() {
        let mut dry = Spu::new();
        let mut wet = Spu::new();
        wet.set_reverb_preset(4);
        wet.set_reverb_depth(0x3000);
        for spu in [&mut dry, &mut wet] {
            spu.upload(0x1000, &looping_block());
            let adsr = Adsr {
                adsr1: 0x00FF,
                adsr2: 0x0000,
            };
            spu.key_on(0, 0x1000, 0x1000, 0x1000, adsr, (0x3FFF, 0x3FFF), true);
        }
        let mut dry_tail = 0i64;
        let mut wet_tail = 0i64;
        for n in 0..60_000 {
            if n == 3000 {
                dry.key_off(0);
                wet.key_off(0);
            }
            let (d, _) = dry.tick();
            let (w, _) = wet.tick();
            if n > 20_000 {
                dry_tail += i64::from(d.abs());
                wet_tail += i64::from(w.abs());
            }
        }
        assert_eq!(dry_tail, 0);
        assert!(wet_tail > 1000, "reverb tail missing: {wet_tail}");
    }
}
