//! The plugin's interpreter over decoded records (`PsxInterpreterRun.cs`):
//! one register bank, thread and pending branch per script bank, a delay
//! slot after every branch, and branch targets with the top byte 0xFF
//! calling the plugin's native functions (`SfxScriptDispatch`).

use crate::gte::Gte;

/// What the interpreter reaches outside its registers: PSX memory
/// (`PsxAddrToPtr`) and the native calls.
pub trait Bus {
    /// Reads `width` (1, 2 or 4) bytes at PSX `address`, zero-extended.
    fn load(&mut self, address: u32, width: u8) -> u32;
    fn store(&mut self, address: u32, width: u8, value: u32);
    /// `SfxScriptDispatch(context, index, 0)` for native call `index` (the
    /// low 22 bits of the target).
    fn native(&mut self, context: &mut Context, index: u32);
}

/// One bank: the thread state at `+3120 + 64 * bank`, LO/HI, the pending
/// branch and the register bank.
#[derive(Debug, Clone, Default)]
pub struct Bank {
    pub records: Vec<[u32; 4]>,
    /// The thread's pc (`+8`): offset of the record running, -4 before the
    /// first; negative ends [`run`].
    pub pc: i32,
    /// Index of the next record (`+16`).
    pub next: usize,
    pub registers: [u32; 32],
    pub lo: u32,
    pub hi: u32,
    pub branch: Option<u32>,
    /// The script block's PSX address and size in bytes (`field_DA8`), for
    /// absolute jump targets.
    pub script_address: u32,
    pub script_size: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Context {
    pub banks: [Bank; 2],
    pub bank: usize,
    pub gte: Gte,
}

impl Context {
    pub fn current(&mut self) -> &mut Bank {
        &mut self.banks[self.bank & 1]
    }

    /// `PsxInterpreterRunJump`.
    pub fn jump(&mut self, bank: usize, offset: u32) {
        let bank = &mut self.banks[bank & 1];
        bank.next = (offset >> 2) as usize;
        bank.pc = offset.wrapping_sub(4) as i32;
    }

    /// Opcode 0x42: restart the current thread at its code base.
    fn restart(&mut self) {
        let bank = self.current();
        bank.pc = -4;
        bank.next = 0;
    }

    /// `PsxInterpreterRunTakeBranch`.
    fn take_branch(&mut self, bus: &mut impl Bus) {
        let bank = self.bank & 1;
        let Some(target) = self.banks[bank].branch.take() else {
            return;
        };
        if target & 0xFF00_0000 == 0xFF00_0000 {
            bus.native(self, target & 0x3F_FFFF);
        } else if (target as i32) >= 0 {
            self.jump(bank, target);
        } else {
            let address = target & 0x0FFF_FFFF;
            let start = self.banks[bank].script_address & 0x0FFF_FFFF;
            let size = self.banks[bank].script_size;
            let offset = if address < start || address >= start.wrapping_add(size) {
                0
            } else {
                address - start
            };
            self.jump(bank, offset);
        }
    }
}

/// Why [`run`] stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The pc left the decoded records.
    PcOutOfRange(usize),
    /// A COP2 command the plugin asserts on.
    Gte(u32),
    /// DIV or DIVU by zero, or `i32::MIN / -1` (the original faults).
    Divide,
}

/// `PsxInterpreterRun`: runs the current bank until its pc goes negative.
/// The register bank is fixed at entry, as in the original.
pub fn run(context: &mut Context, bus: &mut impl Bus) -> Result<(), Fault> {
    let register_bank = context.bank & 1;
    loop {
        let bank = context.bank & 1;
        let thread = &mut context.banks[bank];
        let record = *thread
            .records
            .get(thread.next)
            .ok_or(Fault::PcOutOfRange(thread.next))?;
        thread.pc = thread.pc.wrapping_add(4);
        thread.next += 1;
        step(context, bus, register_bank, record)?;
        if record[0] >> 16 == 0 {
            context.take_branch(bus);
        }
        if context.banks[context.bank & 1].pc < 0 {
            return Ok(());
        }
    }
}

/// `PsxInterpreterRunStep`: one record; opcodes not listed do nothing.
fn step(
    context: &mut Context,
    bus: &mut impl Bus,
    register_bank: usize,
    record: [u32; 4],
) -> Result<(), Fault> {
    let [head, a, b, c] = record;
    let (ai, bi, ci) = ((a & 31) as usize, (b & 31) as usize, (c & 31) as usize);
    let r = |context: &Context, i: usize| context.banks[register_bank].registers[i];
    let set = |context: &mut Context, i: usize, value: u32| {
        if i != 0 {
            context.banks[register_bank].registers[i] = value;
        }
    };
    let link = |context: &mut Context| (context.current().pc as u32).wrapping_add(8);
    let branch = |context: &mut Context, target: u32| context.current().branch = Some(target);
    let address = |context: &Context| r(context, ci).wrapping_add(b);
    match head & 0xFFFF {
        0x01 => set(context, ai, r(context, bi).wrapping_shl(c)),
        0x02 => set(
            context,
            ai,
            ((r(context, bi) as i32).wrapping_shr(c)) as u32,
        ),
        0x03 => set(context, ai, r(context, bi).wrapping_shr(c)),
        0x04 => set(context, ai, r(context, bi)),
        0x05 => {
            let value = link(context);
            set(context, ai, value);
            let target = r(context, bi);
            branch(context, target);
        }
        0x06 => {
            let target = r(context, ai);
            branch(context, target);
        }
        0x07 => {
            let value = context.current().hi;
            set(context, ai, value);
        }
        0x08 => context.current().hi = r(context, ai),
        0x09 => {
            let value = context.current().lo;
            set(context, ai, value);
        }
        0x0A => context.current().lo = r(context, ai),
        0x0B => {
            let product = i64::from(r(context, bi) as i32) * i64::from(r(context, ai) as i32);
            let bank = context.current();
            bank.lo = product as u32;
            bank.hi = (product >> 32) as u32;
        }
        0x0C => {
            let product = u64::from(r(context, bi)) * u64::from(r(context, ai));
            let bank = context.current();
            bank.lo = product as u32;
            bank.hi = (product >> 32) as u32;
        }
        0x0D => {
            let (dividend, divisor) = (r(context, ai) as i32, r(context, bi) as i32);
            let quotient = dividend.checked_div(divisor).ok_or(Fault::Divide)?;
            let remainder = dividend.checked_rem(divisor).ok_or(Fault::Divide)?;
            let bank = context.current();
            bank.lo = quotient as u32;
            bank.hi = remainder as u32;
        }
        0x0E => {
            let (dividend, divisor) = (r(context, ai), r(context, bi));
            let quotient = dividend.checked_div(divisor).ok_or(Fault::Divide)?;
            let bank = context.current();
            bank.lo = quotient;
            bank.hi = dividend % divisor;
        }
        0x0F | 0x10 => set(context, ai, r(context, bi).wrapping_add(r(context, ci))),
        0x11 => set(context, ai, r(context, bi) & r(context, ci)),
        0x12 => set(context, ai, r(context, bi) ^ r(context, ci)),
        // C# masks the shift count to five bits, as wrapping_shr does.
        0x13 => set(context, ai, r(context, bi).wrapping_shr(r(context, ci))),
        0x14 | 0x15 => set(context, ai, r(context, bi).wrapping_sub(r(context, ci))),
        0x16 => set(context, ai, !(r(context, bi) | r(context, ci))),
        0x17 => set(context, ai, r(context, bi) | r(context, ci)),
        0x18 => set(context, ai, r(context, bi).wrapping_shl(r(context, ci))),
        0x19 => set(
            context,
            ai,
            u32::from((r(context, bi) as i32) < (r(context, ci) as i32)),
        ),
        0x1A => set(context, ai, u32::from(r(context, bi) < r(context, ci))),
        0x1B => set(
            context,
            ai,
            (r(context, bi) as i32).wrapping_shr(r(context, ci)) as u32,
        ),
        0x1C | 0x1D => set(context, ai, r(context, bi).wrapping_add(c)),
        0x1E => set(context, ai, u32::from((r(context, bi) as i32) < c as i32)),
        0x1F => set(context, ai, u32::from(r(context, bi) < c)),
        0x20 => set(context, ai, r(context, bi) & c),
        0x21 => set(context, ai, r(context, bi) | c),
        0x22 => set(context, ai, r(context, bi) ^ c),
        0x23 => set(context, ai, b),
        0x24 => set(context, ai, b & 0xFFFF),
        0x25 => set(context, ai, b.wrapping_shl(16)),
        // Loads: the original reads the memory even for r0.
        0x26 => {
            let value = bus.load(address(context), 1) as u8 as i8 as i32 as u32;
            set(context, ai, value);
        }
        0x27 => {
            let value = bus.load(address(context), 2) as u16 as i16 as i32 as u32;
            set(context, ai, value);
        }
        // LWL and LWR are plain 32-bit loads in the original.
        0x28 | 0x29 | 0x2C => {
            let value = bus.load(address(context), 4);
            set(context, ai, value);
        }
        0x2A => {
            let value = bus.load(address(context), 1);
            set(context, ai, value);
        }
        0x2B => {
            let value = bus.load(address(context), 2);
            set(context, ai, value);
        }
        0x2D => bus.store(address(context), 1, r(context, ai)),
        0x2E => bus.store(address(context), 2, r(context, ai)),
        // SWL and SWR are plain 32-bit stores in the original.
        0x2F..=0x31 => bus.store(address(context), 4, r(context, ai)),
        0x32 | 0x3C => branch(context, a),
        0x33 => {
            let value = link(context);
            context.banks[register_bank].registers[31] = value;
            branch(context, a);
        }
        0x34 => {
            if r(context, ai) == r(context, bi) {
                branch(context, c);
            }
        }
        0x35 => {
            if r(context, ai) != r(context, bi) {
                branch(context, c);
            }
        }
        0x36 => {
            if r(context, ai) as i32 >= 0 {
                branch(context, b);
            }
        }
        0x37 => {
            if (r(context, ai) as i32) < 0 {
                branch(context, b);
            }
        }
        0x38 | 0x39 => {
            let negative = (r(context, ai) as i32) < 0;
            if negative == (head & 0xFFFF == 0x38) {
                let value = link(context);
                context.banks[register_bank].registers[31] = value;
                branch(context, b);
            }
        }
        0x3A => {
            if r(context, ai) as i32 <= 0 {
                branch(context, b);
            }
        }
        0x3B => {
            if r(context, ai) as i32 > 0 {
                branch(context, b);
            }
        }
        0x3D => {
            let at = address(context);
            let value = context.gte.read_data(ai);
            bus.store(at, 4, value);
        }
        0x3E => {
            let value = bus.load(address(context), 4);
            context.gte.write_data(ai, value);
        }
        0x3F => set(context, ai, context.gte.control[bi]),
        0x40 => context.gte.control[bi] = r(context, ai),
        0x41 => {
            if !context.gte.command(a) {
                return Err(Fault::Gte(a));
            }
        }
        0x42 => context.restart(),
        0x59 => {
            let value = context.gte.read_data(bi);
            set(context, ai, value);
        }
        0x5A => {
            let value = r(context, ai);
            context.gte.write_data(bi, value);
        }
        _ => {}
    }
    Ok(())
}
