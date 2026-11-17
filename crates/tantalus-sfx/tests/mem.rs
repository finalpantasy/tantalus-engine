//! The flat memory and the PSX address mapper on a blank image.

use tantalus_sfx::mem::{
    HEAP_BASE, IMAGE_BASE, MAPPER, Memory, host_to_psx, psx_to_host, register, select_ram,
};

fn memory() -> Memory {
    Memory::with_image_size(0x60_0000)
}

#[test]
fn heap_and_image_round_trip() {
    let mut memory = memory();
    let block = memory.alloc(10);
    assert_eq!(block, HEAP_BASE);
    assert_eq!(memory.alloc(1), HEAP_BASE + 16);
    memory.set_u32(block + 4, 0xDEAD_BEEF).unwrap();
    assert_eq!(memory.u16(block + 6).unwrap(), 0xDEAD);
    memory.set_u64(IMAGE_BASE + 8, block).unwrap();
    assert_eq!(memory.u64(IMAGE_BASE + 8).unwrap(), block);
    assert!(memory.u8(0x1000).is_err());
}

/// RAM maps 0x80xxxxxx, a registered window maps (slot << 24) | 0xC00000
/// from 128 KB below its start, and the scratchpad sits in the mapper.
#[test]
fn mapper_round_trips() {
    let mut memory = memory();
    let ram = memory.alloc(0x20_0000);
    select_ram(&mut memory, MAPPER, 0x8001_0000, ram).unwrap();
    assert_eq!(
        psx_to_host(&memory, MAPPER, 0x8001_0040).unwrap(),
        ram + 0x40
    );
    let object = memory.alloc(64);
    let psx = register(&mut memory, MAPPER, object + 8, false).unwrap();
    assert_eq!(psx, 0x00C2_0000);
    assert_eq!(psx_to_host(&memory, MAPPER, psx).unwrap(), object + 8);
    // A second address in the same window reuses it.
    assert_eq!(
        register(&mut memory, MAPPER, object + 24, true).unwrap(),
        0x00C2_0010
    );
    assert_eq!(
        host_to_psx(&memory, MAPPER, object + 24).unwrap(),
        0x00C2_0010
    );
    assert_eq!(
        psx_to_host(&memory, MAPPER, 0x1F80_0010).unwrap(),
        MAPPER + 8152 + 0x10
    );
    assert_eq!(psx_to_host(&memory, MAPPER, 0x1234).unwrap(), 0);
}
