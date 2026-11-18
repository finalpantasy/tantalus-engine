//! A stand-in host, the same as the test oracle's (Memoria's
//! `SFX.DebugRoomCallback`, `SFX.cs:1401`): VRAM calls go to a [`Vram`], unit
//! queries answer with one fixed position and identity transforms, the rest
//! return 0. Every call is logged, so a run can be compared with the DLL's.

use super::rt::{Host, Memory};
use crate::vram::{Rect, Vram};

/// One host callback the plugin made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// The frame (`SFX_Update` calls so far) it was made in.
    pub frame: u32,
    /// `code >> 24`.
    pub command: u8,
    /// `code & 0xFFFFFF` (the object id for unit commands).
    pub low: u32,
    pub args: [i32; 4],
    /// FNV-1a of the pixels of a `LoadImage`.
    pub pixels: Option<u64>,
}

/// FNV-1a over the little-endian bytes of `words`.
pub fn fnv(words: &[u16]) -> u64 {
    words.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, word| {
        word.to_le_bytes().iter().fold(hash, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        })
    })
}

/// Where the stand-in puts battle object `btl_id`: party members in front of
/// the camera, enemies behind them.
pub fn position(btl_id: u32) -> [i16; 3] {
    let slot = btl_id.trailing_zeros() as i16;
    if btl_id >= 0x10 {
        [(slot - 4) * 400, 0, -1000]
    } else {
        [slot * 400 - 600, 0, 1000]
    }
}

/// GetMatrix mode 2 (`SFX.cs` battle callback case 14, "Get Bone Orientation &
/// Position"): a whole `PSXMAT` (r, pad, t), here a missing bone
/// (`Matrix4x4.identity`) at the unit's position. The host fills all 32 bytes;
/// the plugin does not clear its buffer first, so a host that writes less
/// leaves the DLL reading its own stack (effects 341-346 did).
pub fn matrix(btl_id: u32) -> [u8; 32] {
    let r = crate::player::Bone::IDENTITY.r;
    let t = position(btl_id).map(i32::from);
    let mut out = [0u8; 32];
    for (i, value) in r.iter().enumerate() {
        out[2 * i..2 * i + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (i, value) in t.iter().enumerate() {
        out[20 + 4 * i..24 + 4 * i].copy_from_slice(&value.to_le_bytes());
    }
    out
}

#[derive(Debug, Default)]
pub struct DebugHost {
    pub vram: Vram,
    pub calls: Vec<Call>,
    pub frame: u32,
}

fn words(memory: &Memory, at: u64, count: usize) -> Vec<u16> {
    memory
        .bytes(at, count * 2)
        .map(|bytes| {
            bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect()
        })
        .unwrap_or_default()
}

fn write_i16(memory: &mut Memory, at: u64, values: &[i16]) {
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let _ = memory.write_bytes(at, &bytes);
}

impl DebugHost {
    /// The answer to one callback (`SFX.Callback`), without logging.
    pub fn answer(
        &mut self,
        memory: &mut Memory,
        code: i32,
        args: [i32; 4],
        p: u64,
    ) -> (i32, Option<u64>) {
        let command = (code as u32 >> 24) as u8;
        let [a0, a1, a2, a3] = args;
        let rect = Rect {
            x: a0 as i16,
            y: a1 as i16,
            w: a2 as i16,
            h: a3 as i16,
        };
        match command {
            100 if p != 0 => {
                let count = (i32::from(rect.w).max(0) * i32::from(rect.h).max(0)) as usize;
                let pixels = words(memory, p, count);
                let hash = fnv(&pixels);
                self.vram.load_image(rect, &pixels);
                (0, Some(hash))
            }
            101 if p != 0 => {
                let pixels = self.vram.store_image(rect);
                let bytes: Vec<u8> = pixels.iter().flat_map(|v| v.to_le_bytes()).collect();
                let _ = memory.write_bytes(p, &bytes);
                (0, None)
            }
            102 if p != 0 => {
                let source = words(memory, p, 4);
                self.vram.move_image(
                    Rect {
                        x: source[0] as i16,
                        y: source[1] as i16,
                        w: source[2] as i16,
                        h: source[3] as i16,
                    },
                    a0 as i16,
                    a1 as i16,
                );
                (0, None)
            }
            1 if p != 0 => {
                let at = position(code as u32 & 0xFF);
                let value = if a0 >= 2 { [at[0], 0, at[2]] } else { at };
                write_i16(memory, p, &value);
                (0, None)
            }
            3 if p != 0 => {
                write_i16(memory, p, &[0; 3]);
                (0, None)
            }
            5 if p != 0 => {
                let bytes: Vec<u8> = [4096i32; 3].iter().flat_map(|v| v.to_le_bytes()).collect();
                let _ = memory.write_bytes(p, &bytes);
                (0, None)
            }
            9 | 15 => (1, None),
            14 => {
                let at = position(code as u32 & 0xFF);
                if a0 == 1 {
                    return (i32::from(at[1]), None);
                }
                if p != 0 && a0 == 2 {
                    let _ = memory.write_bytes(p, &matrix(code as u32 & 0xFF));
                } else if p != 0 {
                    write_i16(memory, p, &at);
                }
                (0, None)
            }
            _ => (0, None),
        }
    }
}

impl Host for DebugHost {
    fn callback(
        &mut self,
        memory: &mut Memory,
        code: i32,
        a0: i32,
        a1: i32,
        a2: i32,
        a3: i32,
        p: u64,
    ) -> i32 {
        let (result, pixels) = self.answer(memory, code, [a0, a1, a2, a3], p);
        self.calls.push(Call {
            frame: self.frame,
            command: (code as u32 >> 24) as u8,
            low: code as u32 & 0xFF_FFFF,
            args: [a0, a1, a2, a3],
            pixels,
        });
        result
    }
}
