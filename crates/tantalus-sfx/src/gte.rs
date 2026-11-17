//! The plugin's GTE subset (`GteRtps.cs`, `PsxInterpreterRun.cs:400-470`):
//! the register file at `0x180211F40` (32 control words, then 32 data words
//! at `0x180211FC0`) and the six COP2 commands effect scripts use. The
//! plugin's arithmetic differs from the PS1 hardware in places (no flag
//! register, AVSZ3 averages SZ0..SZ2 and divides by 3, RTPS's MAC0 uses the
//! address of DQA); this port keeps the plugin's results.

/// Low 32 bits of the address of DQA (`0x180211FAC`) when the plugin sits at
/// its preferred base: RTPS multiplies by the address, not the register
/// (`GteRtps.cs`, the LEA at 0x180004078).
pub const DQA_ADDRESS: u32 = 0x8021_1FAC;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gte {
    pub control: [u32; 32],
    pub data: [u32; 32],
    /// The value RTPS multiplies `q` by; [`DQA_ADDRESS`] by default.
    pub dqa_address: u32,
}

impl Default for Gte {
    fn default() -> Self {
        Self {
            control: [0; 32],
            data: [0; 32],
            dqa_address: DQA_ADDRESS,
        }
    }
}

fn clamp_s16(value: i32) -> i16 {
    value.clamp(-32768, 0x7FFF) as i16
}

fn clamp_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

fn low(word: u32) -> i16 {
    word as u16 as i16
}

fn high(word: u32) -> i16 {
    (word >> 16) as u16 as i16
}

fn set_low(word: &mut u32, value: i16) {
    *word = (*word & 0xFFFF_0000) | u32::from(value as u16);
}

fn set_high(word: &mut u32, value: i16) {
    *word = (*word & 0xFFFF) | (u32::from(value as u16) << 16);
}

// Data register numbers.
const RGBC: usize = 6;
const OTZ: usize = 7;
const IR0: usize = 8;
const IR1: usize = 9;
const IR2: usize = 10;
const IR3: usize = 11;
const SXY0: usize = 12;
const SZ0: usize = 16;
const RGB0: usize = 20;
const MAC0: usize = 24;
const MAC1: usize = 25;
const LZCS: usize = 30;
const LZCR: usize = 31;

impl Gte {
    /// The rotation matrix as nine s16 (`xmmword_180211F40`, RT33 in the
    /// low half of control word 4).
    fn rotation(&self) -> [i32; 9] {
        let c = &self.control;
        [
            low(c[0]),
            high(c[0]),
            low(c[1]),
            high(c[1]),
            low(c[2]),
            high(c[2]),
            low(c[3]),
            high(c[3]),
            low(c[4]),
        ]
        .map(i32::from)
    }

    /// `GteReadData` (MFC2/SWC2): VZ0..VZ2 and IR0..IR3 are first
    /// sign-extended in place from their low 16 bits.
    pub fn read_data(&mut self, index: usize) -> u32 {
        let index = index & 31;
        if matches!(index, 1 | 3 | 5 | 8..=11) {
            self.data[index] = i32::from(low(self.data[index])) as u32;
        }
        self.data[index]
    }

    /// `PsxInterpreterRunGteWrite` (MTC2/LWC2): writing LZCS stores its
    /// leading-sign-bit count in LZCR first.
    pub fn write_data(&mut self, index: usize, value: u32) {
        let index = index & 31;
        if index == LZCS {
            let value = value as i32;
            let bits = if value >= 0 { !value } else { value };
            self.data[LZCR] = bits.leading_ones();
        }
        self.data[index] = value;
    }

    /// `PsxInterpreterRunGteCommand`; `false` for a command the plugin
    /// asserts on (`PsxEmulator.cpp` line 0x4E7).
    pub fn command(&mut self, command: u32) -> bool {
        match command {
            0x78_0010 => self.dpcs(),
            0x18_0001 => self.rtps(0),
            0x28_0030 => {
                self.rtps(0);
                self.rtps(1);
                self.rtps(2);
                self.data[MAC0] = 1;
            }
            0x48_0012 => self.rtv0tr(),
            0x140_0006 => self.nclip(),
            0x158_002D => self.avsz3(),
            _ => return false,
        }
        true
    }

    /// The three products of row `row` of RT with `v` (x, y, z), in 32 bits.
    fn row(rt: &[i32; 9], row: usize, v: [i32; 3]) -> i32 {
        (v[0].wrapping_mul(rt[row * 3]))
            .wrapping_add(v[1].wrapping_mul(rt[row * 3 + 1]))
            .wrapping_add(v[2].wrapping_mul(rt[row * 3 + 2]))
    }

    /// MAC1..3 = TR + (RT * v) >> 12 and IR1..3 clamped to s16.
    fn transform(&mut self, v: [i32; 3]) -> [i32; 3] {
        let rt = self.rotation();
        let mut mac = [0; 3];
        for (row, value) in mac.iter_mut().enumerate() {
            *value = (self.control[5 + row] as i32).wrapping_add(Self::row(&rt, row, v) >> 12);
            self.data[MAC1 + row] = *value as u32;
        }
        set_low(&mut self.data[IR1], clamp_s16(mac[0]));
        set_low(&mut self.data[IR2], clamp_s16(mac[1]));
        set_low(&mut self.data[IR3], clamp_s16(mac[2]));
        mac
    }

    fn vector(&self, index: usize) -> [i32; 3] {
        let xy = self.data[index * 2];
        let z = self.data[index * 2 + 1];
        [low(xy), high(xy), low(z)].map(i32::from)
    }

    /// `GteRtV0Tr`.
    fn rtv0tr(&mut self) {
        self.transform(self.vector(0));
    }

    /// `GteRtps` of vector `index`.
    fn rtps(&mut self, index: usize) {
        let mac = self.transform(self.vector(index));
        let (ir1, ir2) = (i64::from(clamp_s16(mac[0])), i64::from(clamp_s16(mac[1])));
        self.data[SZ0] = self.data[SZ0 + 1];
        self.data[SZ0 + 1] = self.data[SZ0 + 2];
        self.data[SZ0 + 2] = self.data[SZ0 + 3];
        let sz3 = mac[2].clamp(0, 0xFFFF) as u16;
        self.data[SZ0 + 3] = (self.data[SZ0 + 3] & 0xFFFF_0000) | u32::from(sz3);
        self.data[SXY0] = self.data[SXY0 + 1];
        self.data[SXY0 + 1] = self.data[SXY0 + 2];
        let q = if sz3 != 0 {
            (i32::from(low(self.control[26])) << 16) / i32::from(sz3)
        } else {
            0x1FFFF
        };
        let screen = |offset: u32, ir: i64| {
            let value = (i64::from(offset as i32) + ((i64::from(q) * ir) >> 16)) as i32;
            value.clamp(-1024, 1023) as i16
        };
        let sx = screen(self.control[24], ir1);
        let sy = screen(self.control[25], ir2);
        set_low(&mut self.data[SXY0 + 2], sx);
        set_high(&mut self.data[SXY0 + 2], sy);
        let mac0 =
            (self.control[28] as i32).wrapping_add((self.dqa_address as i32).wrapping_mul(q));
        self.data[MAC0] = mac0 as u32;
        set_low(&mut self.data[IR0], (mac0 >> 12).clamp(0, 4095) as i16);
    }

    /// NCLIP over SXY0..SXY2, in 32 bits.
    fn nclip(&mut self) {
        let [sx0, sx1, sx2] = [0, 1, 2].map(|i| i32::from(low(self.data[SXY0 + i])));
        let [sy0, sy1, sy2] = [0, 1, 2].map(|i| i32::from(high(self.data[SXY0 + i])));
        let value = (sy2.wrapping_mul(sx1))
            .wrapping_sub(sy2.wrapping_mul(sx0))
            .wrapping_sub(sx1.wrapping_mul(sy0))
            .wrapping_sub(sy1.wrapping_mul(sx2))
            .wrapping_add(sx0.wrapping_mul(sy1))
            .wrapping_add(sy0.wrapping_mul(sx2));
        self.data[MAC0] = value as u32;
    }

    /// `GteAvsz3`: MAC0 = ((SZ0 + SZ1 + SZ2) << 12) / 3, OTZ = MAC0 >> 14.
    fn avsz3(&mut self) {
        let sum: i32 = (0..3).map(|i| i32::from(self.data[SZ0 + i] as u16)).sum();
        let mac0 = sum.wrapping_shl(12) / 3;
        self.data[MAC0] = mac0 as u32;
        let otz = (mac0 >> 14).clamp(0, 0xFFFF);
        set_low(&mut self.data[OTZ], otz as u16 as i16);
    }

    /// `GteDpcs`: depth cue of RGBC toward FC by IR0.
    fn dpcs(&mut self) {
        let rgbc = self.data[RGBC].to_le_bytes();
        let ir0 = i64::from(low(self.data[IR0]));
        let mut mac = [0i32; 3];
        for (channel, value) in mac.iter_mut().enumerate() {
            let color = i32::from(rgbc[channel]);
            let far = (self.control[21 + channel] as i32).wrapping_sub(16 * color);
            *value = ((i64::from(color << 16) + ir0 * i64::from(far)) >> 12) as i32;
            self.data[MAC1 + channel] = *value as u32;
        }
        set_low(&mut self.data[IR1], clamp_s16(mac[0]));
        set_low(&mut self.data[IR2], clamp_s16(mac[1]));
        set_low(&mut self.data[IR3], clamp_s16(mac[2]));
        self.data[RGB0] = self.data[RGB0 + 1];
        self.data[RGB0 + 1] = self.data[RGB0 + 2];
        self.data[RGB0 + 2] = u32::from_le_bytes([
            clamp_u8(mac[0] >> 4),
            clamp_u8(mac[1] >> 4),
            clamp_u8(mac[2] >> 4),
            rgbc[3],
        ]);
    }
}
