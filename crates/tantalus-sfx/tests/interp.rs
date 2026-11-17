//! The interpreter on hand-built records (opcode index | delay-slot flag
//! << 16, then three operands, as `MipsDecodeAll` lays them out).

use tantalus_sfx::interp::{Bus, Context, run};

#[derive(Default)]
struct Ram {
    bytes: Vec<u8>,
    natives: Vec<u32>,
}

impl Bus for Ram {
    fn load(&mut self, address: u32, width: u8) -> u32 {
        let at = (address & 0xFFFF) as usize;
        let mut value = 0;
        for i in 0..usize::from(width) {
            value |= u32::from(self.bytes[at + i]) << (8 * i);
        }
        value
    }

    fn store(&mut self, address: u32, width: u8, value: u32) {
        let at = (address & 0xFFFF) as usize;
        for i in 0..usize::from(width) {
            self.bytes[at + i] = (value >> (8 * i)) as u8;
        }
    }

    /// Native 7 ends the thread, as a yielding native call does.
    fn native(&mut self, context: &mut Context, index: u32) {
        self.natives.push(index);
        if index == 7 {
            context.current().pc = -1;
        }
    }
}

const BRANCH: u32 = 1 << 16;

/// Sums 1..=5 with a BNE loop (its delay slot runs every pass), stores the
/// sum with SW, loads it back with LW, then calls native 7 through JR.
#[test]
fn loop_store_load_and_native_call() {
    let records = vec![
        [0x1D, 8, 0, 5],             // 0x00 addiu t0, r0, 5
        [0x1D, 9, 0, 0],             // 0x04 addiu t1, r0, 0
        [0x10, 9, 9, 8],             // 0x08 addu t1, t1, t0
        [0x35 | BRANCH, 8, 0, 0x08], // 0x0C bne t0, r0, 0x08
        [0x1D, 8, 8, u32::MAX],      // 0x10 addiu t0, t0, -1 (delay slot)
        [0x30, 9, 0x40, 0],          // 0x14 sw t1, 0x40(r0)
        [0x29, 10, 0x40, 0],         // 0x18 lw t2, 0x40(r0)
        [0x25, 11, 0xFF00, 0],       // 0x1C lui t3, 0xFF00
        [0x21, 11, 11, 7],           // 0x20 ori t3, t3, 7
        [0x06 | BRANCH, 11, 0, 0],   // 0x24 jr t3
        [0x04, 12, 10, 0],           // 0x28 move t4, t2 (delay slot)
        [0x04, 13, 10, 0],           // 0x2C never runs
    ];
    let mut context = Context::default();
    context.banks[0].records = records;
    context.banks[0].pc = -4;
    let mut ram = Ram {
        bytes: vec![0; 0x100],
        ..Ram::default()
    };
    run(&mut context, &mut ram).expect("run");
    let registers = context.banks[0].registers;
    // The loop adds t0 before the delay slot decrements it: 5+4+3+2+1+0.
    assert_eq!(registers[9], 15);
    assert_eq!(registers[10], 15);
    assert_eq!(registers[12], 15);
    assert_eq!(registers[13], 0);
    assert_eq!(ram.natives, vec![7]);
}

/// JAL links pc + 8 in r31; r0 stays zero.
#[test]
fn jal_links_and_r0_stays_zero() {
    let records = vec![
        [0x1D, 0, 0, 9],             // 0x00 addiu r0, r0, 9 (ignored)
        [0x33 | BRANCH, 0x10, 0, 0], // 0x04 jal 0x10
        [0x04, 0, 0, 0],             // 0x08 nop (delay slot)
        [0x04, 0, 0, 0],             // 0x0C skipped
        [0x23, 5, 0xFF00_0007, 0],   // 0x10 li a1, 0xFF000007
        [0x06 | BRANCH, 5, 0, 0],    // 0x14 jr a1
        [0x04, 0, 0, 0],             // 0x18 delay slot
    ];
    let mut context = Context::default();
    context.banks[0].records = records;
    context.banks[0].pc = -4;
    let mut ram = Ram::default();
    run(&mut context, &mut ram).expect("run");
    assert_eq!(context.banks[0].registers[0], 0);
    assert_eq!(context.banks[0].registers[31], 0x0C);
}
