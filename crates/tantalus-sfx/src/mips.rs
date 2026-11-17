//! The plugin's MIPS front end: `MipsDecodeAll` turns each instruction word
//! of an effect script into a 16-byte record (opcode index, delay-slot
//! flag, three operands) that the interpreter runs
//! (`SfxBuildRotScaleMatrix.cs:181`, `PsxInterpreterRun.cs`).

use crate::image::PluginImage;

/// RVA of the opcode table (`dword_180066C70`): 56-byte entries of mask,
/// value, name pointer, up to four operand decoder pointers at +16 and a
/// byte at +48, ended by an entry whose mask is zero.
const OPCODE_TABLE: u32 = 0x66C70;
const ENTRY_SIZE: usize = 56;

/// The operand decoders the table points at, by RVA
/// (`InitE7Packet.cs:105-166`, `SFX_SendIntData_0.cs:135-158`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operand {
    /// `MipsDecodeRs` (0xD0D0): bits 21..25.
    Rs,
    /// `MipsDecodeRt` (0xD0E0): bits 16..20.
    Rt,
    /// `MipsDecodeRd` (0xD0F0): bits 11..15.
    Rd,
    /// `MipsDecodeShamt` (0xD100): bits 6..10.
    Shamt,
    /// `MipsDecodeCode20` (0xD110): bits 6..25.
    Code20,
    /// `MipsDecodeSimm16` (0xD120): the sign-extended immediate.
    Simm16,
    /// `MipsDecodeImm16` (0xD130): the zero-extended immediate.
    Imm16,
    /// `MipsJumpTargetOffset` (0xD140): index << 2, minus the script's
    /// address in `MipsDecodeAll`.
    JumpTarget,
    /// `MipsDecodeImm25` (0xD150): the COP2 command.
    Imm25,
    /// `MipsBranchTarget` (0xD160): pc + 4 * (simm16 + 1).
    BranchTarget,
    /// `MipsDecodeImm16Rs` (0xD180): the sign-extended immediate, then rs
    /// as a second operand.
    Imm16Rs,
}

impl Operand {
    fn from_rva(rva: u32) -> Option<Self> {
        Some(match rva {
            0xD0D0 => Self::Rs,
            0xD0E0 => Self::Rt,
            0xD0F0 => Self::Rd,
            0xD100 => Self::Shamt,
            0xD110 => Self::Code20,
            0xD120 => Self::Simm16,
            0xD130 => Self::Imm16,
            0xD140 => Self::JumpTarget,
            0xD150 => Self::Imm25,
            0xD160 => Self::BranchTarget,
            0xD180 => Self::Imm16Rs,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
struct Entry {
    mask: u32,
    value: u32,
    operands: Vec<Operand>,
    flag: u8,
}

/// The opcode table, read from the player's plugin.
#[derive(Debug, Clone)]
pub struct OpcodeTable {
    entries: Vec<Entry>,
    /// The terminating entry's byte at +48.
    end_flag: u8,
}

impl OpcodeTable {
    /// Reads the table; `None` when it is missing or points at a decoder
    /// this port does not know.
    pub fn read(image: &PluginImage) -> Option<Self> {
        let mut entries = Vec::new();
        for index in 0..256u32 {
            let bytes = image.read(OPCODE_TABLE + index * ENTRY_SIZE as u32, ENTRY_SIZE)?;
            let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            let mask = word(0);
            let flag = bytes[48];
            if mask == 0 {
                return Some(Self {
                    entries,
                    end_flag: flag,
                });
            }
            let mut operands = Vec::new();
            for slot in 0..4 {
                let at = 16 + slot * 8;
                let va = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
                if va == 0 {
                    break;
                }
                operands.push(Operand::from_rva(image.rva(va)?)?);
            }
            entries.push(Entry {
                mask,
                value: word(4),
                operands,
                flag,
            });
        }
        None
    }

    /// How many operand slots opcode `index` writes (the rest keep the
    /// previous record's values).
    pub fn operand_count(&self, index: usize) -> usize {
        self.entries.get(index).map_or(0, |entry| {
            entry
                .operands
                .iter()
                .map(|&operand| 1 + usize::from(operand == Operand::Imm16Rs))
                .sum::<usize>()
                .min(3)
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One bank's decode state. The operand words live in the thread state
/// (+52..+63) and are not cleared between instructions, so an opcode with
/// fewer operands keeps the previous instruction's values in the unused
/// slots, as the original's records do.
#[derive(Debug, Clone, Default)]
pub struct Decoder {
    operands: [u32; 4],
}

impl Decoder {
    /// `MipsDecodeAll` for the script at PSX `address` (already masked to
    /// 28 bits, `SfxLoaderStep.cs:420`): one record per word, `[index |
    /// flag << 16, operand 0, operand 1, operand 2]`.
    pub fn decode(&mut self, table: &OpcodeTable, address: u32, words: &[u32]) -> Vec<[u32; 4]> {
        let mut out = Vec::with_capacity(words.len());
        for (at, &word) in words.iter().enumerate() {
            let pc = (at as u32) * 4;
            let found = table
                .entries
                .iter()
                .position(|entry| word & entry.mask == entry.value);
            let head = match found {
                Some(index) => {
                    let entry = &table.entries[index];
                    let mut slot = 0;
                    for &operand in &entry.operands {
                        let (value, extra) = operand_value(operand, pc, word);
                        let value = if operand == Operand::JumpTarget {
                            value.wrapping_sub(address)
                        } else {
                            value
                        };
                        if slot < 4 {
                            self.operands[slot] = value;
                        }
                        slot += 1;
                        if let Some(extra) = extra {
                            if slot < 4 {
                                self.operands[slot] = extra;
                            }
                            slot += 1;
                        }
                    }
                    index as u32 | u32::from(entry.flag) << 16
                }
                None => table.entries.len() as u32 | u32::from(table.end_flag) << 16,
            };
            out.push([head, self.operands[0], self.operands[1], self.operands[2]]);
        }
        out
    }
}

/// One decoder's result, and the pair decoder's second value.
fn operand_value(operand: Operand, pc: u32, word: u32) -> (u32, Option<u32>) {
    let simm = word as u16 as i16 as i32 as u32;
    let value = match operand {
        Operand::Rs => (word >> 21) & 0x1F,
        Operand::Rt => (word >> 16) & 0x1F,
        Operand::Rd => (word >> 11) & 0x1F,
        Operand::Shamt => (word >> 6) & 0x1F,
        Operand::Code20 => (word >> 6) & 0xF_FFFF,
        Operand::Simm16 => simm,
        Operand::Imm16 => word & 0xFFFF,
        Operand::JumpTarget => (word & 0x3FF_FFFF) << 2,
        Operand::Imm25 => word & 0x1FF_FFFF,
        Operand::BranchTarget => pc.wrapping_add(simm.wrapping_add(1).wrapping_mul(4)),
        Operand::Imm16Rs => return (simm, Some((word >> 21) & 0x1F)),
    };
    (value, None)
}
