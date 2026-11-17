//! The plugin's memory as one flat 64-bit address space, so the port can
//! follow Memoria's pointer arithmetic line for line: the module image at
//! its preferred base (data sections copied from the player's DLL, BSS
//! zeroed), and a heap for what the plugin allocates. Host pointers stored
//! in this memory are addresses in this space.
//!
//! The PSX address mapper (`PsxVirtualAddrMapper64`, the global at
//! `0x180576A10`) is translated over it: `PsxAddrToPtr`
//! (`HostCallbackSend.cs:43`), `PsxPtrToAddr` (`PsxPtrToAddr.cs`) and
//! `PsxAddrMapperRegister` (`PsxClearOTagR.cs`).

use crate::image::PluginImage;

/// The DLL's preferred base; the pointers in its data sections assume it.
pub const IMAGE_BASE: u64 = 0x1_8000_0000;
/// Where [`Memory::alloc`] hands out blocks.
pub const HEAP_BASE: u64 = 0x2_0000_0000;
/// The distance between heap blocks (see [`Memory::alloc`]).
pub const HEAP_SPACING: u64 = 4 << 20;
/// Where the port's native stack lives (`stackalloc` and locals whose
/// address the rewrite takes).
pub const STACK_BASE: u64 = 0x40_0000_0000;
/// Size of that stack.
pub const STACK_SIZE: usize = 4 << 20;
/// `unk_180576A10`: the global mapper.
pub const MAPPER: u64 = IMAGE_BASE + 0x57_6A10;

/// A read or write outside every region (the original would fault).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fault(pub u64);

#[derive(Clone)]
pub struct Memory {
    image: Vec<u8>,
    /// Heap blocks by start address.
    heap: std::collections::BTreeMap<u64, Vec<u8>>,
    heap_next: u64,
    stack: Vec<u8>,
}

impl std::fmt::Debug for Memory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Memory")
            .field("image", &self.image.len())
            .field("heap", &self.heap.len())
            .finish()
    }
}

impl Memory {
    /// The module image of `dll`: every section at its RVA. `None` when the
    /// image has no sections.
    pub fn from_image(dll: &PluginImage) -> Option<Self> {
        let end = dll.image_end()?;
        let mut image = vec![0u8; end as usize];
        for (rva, size) in dll.section_ranges() {
            if let Some(bytes) = dll.read(rva, size as usize) {
                image[rva as usize..rva as usize + bytes.len()].copy_from_slice(&bytes);
            }
        }
        Some(Self {
            image,
            heap: Default::default(),
            heap_next: HEAP_BASE,
            stack: vec![0; STACK_SIZE],
        })
    }

    /// An empty image of `size` bytes (for tests).
    pub fn with_image_size(size: usize) -> Self {
        Self {
            image: vec![0; size],
            heap: Default::default(),
            heap_next: HEAP_BASE,
            stack: vec![0; STACK_SIZE],
        }
    }

    /// `NativeMemory.Alloc`: a zeroed block. Blocks are never reused, so a
    /// freed pointer cannot alias a new one, and each starts on its own
    /// [`HEAP_SPACING`] boundary: the original's heap blocks are far apart,
    /// so each gets its own window in the PSX address mapper (a window
    /// covers 3 MB), and the port must open the same windows.
    pub fn alloc(&mut self, size: usize) -> u64 {
        let start = self.heap_next;
        let size = size.max(1);
        self.heap.insert(start, vec![0; size]);
        self.heap_next = (start + size as u64 + HEAP_SPACING).next_multiple_of(HEAP_SPACING);
        start
    }

    fn slice(&self, address: u64, len: usize) -> Result<&[u8], Fault> {
        let (region, offset) = if address >= STACK_BASE {
            (&self.stack, address - STACK_BASE)
        } else if address >= HEAP_BASE {
            let (start, block) = self
                .heap
                .range(..=address)
                .next_back()
                .ok_or(Fault(address))?;
            (block, address - start)
        } else if address >= IMAGE_BASE {
            (&self.image, address - IMAGE_BASE)
        } else {
            return Err(Fault(address));
        };
        let offset = usize::try_from(offset).map_err(|_| Fault(address))?;
        region.get(offset..offset + len).ok_or(Fault(address))
    }

    fn slice_mut(&mut self, address: u64, len: usize) -> Result<&mut [u8], Fault> {
        let (region, offset) = if address >= STACK_BASE {
            (&mut self.stack, address - STACK_BASE)
        } else if address >= HEAP_BASE {
            let (start, block) = self
                .heap
                .range_mut(..=address)
                .next_back()
                .ok_or(Fault(address))?;
            (block, address - start)
        } else if address >= IMAGE_BASE {
            (&mut self.image, address - IMAGE_BASE)
        } else {
            return Err(Fault(address));
        };
        let offset = usize::try_from(offset).map_err(|_| Fault(address))?;
        region.get_mut(offset..offset + len).ok_or(Fault(address))
    }

    pub fn bytes(&self, address: u64, len: usize) -> Result<&[u8], Fault> {
        self.slice(address, len)
    }

    pub fn write_bytes(&mut self, address: u64, bytes: &[u8]) -> Result<(), Fault> {
        self.slice_mut(address, bytes.len())?.copy_from_slice(bytes);
        Ok(())
    }

    /// `memmove`: copies `len` bytes from `source` to `destination`.
    pub fn copy(&mut self, destination: u64, source: u64, len: usize) {
        if len == 0 {
            return;
        }
        let bytes = self
            .bytes(source, len)
            .unwrap_or_else(|f| panic!("read fault at {:#x}", f.0))
            .to_vec();
        self.write_bytes(destination, &bytes)
            .unwrap_or_else(|f| panic!("write fault at {:#x}", f.0));
    }

    /// `memset`.
    pub fn fill(&mut self, destination: u64, value: u8, len: usize) {
        if len == 0 {
            return;
        }
        self.slice_mut(destination, len)
            .unwrap_or_else(|f| panic!("write fault at {:#x}", f.0))
            .fill(value);
    }

    pub fn u8(&self, address: u64) -> Result<u8, Fault> {
        Ok(self.slice(address, 1)?[0])
    }

    pub fn u16(&self, address: u64) -> Result<u16, Fault> {
        Ok(u16::from_le_bytes(
            self.slice(address, 2)?.try_into().unwrap(),
        ))
    }

    pub fn u32(&self, address: u64) -> Result<u32, Fault> {
        Ok(u32::from_le_bytes(
            self.slice(address, 4)?.try_into().unwrap(),
        ))
    }

    pub fn u64(&self, address: u64) -> Result<u64, Fault> {
        Ok(u64::from_le_bytes(
            self.slice(address, 8)?.try_into().unwrap(),
        ))
    }

    pub fn set_u8(&mut self, address: u64, value: u8) -> Result<(), Fault> {
        self.write_bytes(address, &[value])
    }

    pub fn set_u16(&mut self, address: u64, value: u16) -> Result<(), Fault> {
        self.write_bytes(address, &value.to_le_bytes())
    }

    pub fn set_u32(&mut self, address: u64, value: u32) -> Result<(), Fault> {
        self.write_bytes(address, &value.to_le_bytes())
    }

    pub fn set_u64(&mut self, address: u64, value: u64) -> Result<(), Fault> {
        self.write_bytes(address, &value.to_le_bytes())
    }
}

// PsxAddrMapper (Structs.g.cs): 254 entries of 32 bytes (used u8 @0,
// index u8 @1, base u64 @8, start u64 @16, end u64 @24), count i32 @8128,
// ramPsxAddress u32 @8136, ramHostBase u64 @8144, scratchpad @8152.
const ENTRY: u64 = 32;
const COUNT: u64 = 8128;
const RAM_PSX_ADDRESS: u64 = 8136;
const RAM_HOST_BASE: u64 = 8144;
const SCRATCHPAD: u64 = 8152;
/// A registered window covers 3 MB from its start; its base is 128 KB
/// below the start.
const WINDOW: u64 = 0x30_0000;
const RAM_ENTRY: u64 = 128;

/// `PsxAddrToPtr`: the host address of PSX address `psx`, or 0. RAM
/// (0x80xxxxxx below 2 MB), registered windows (bits 22 and 23 set) and the
/// 1 KB scratchpad at 0x1F800000.
pub fn psx_to_host(memory: &Memory, mapper: u64, psx: u32) -> Result<u64, Fault> {
    if psx == 0 {
        return Ok(0);
    }
    let segment = u64::from(psx >> 24);
    if segment == 128 {
        if psx & 0x0FFF_FFFF < 0x20_0000 {
            let base = memory.u64(mapper + RAM_HOST_BASE)?;
            let psx_base = memory.u32(mapper + RAM_PSX_ADDRESS)?;
            // 32-bit subtraction, then zero-extended (PSX-ADDR-001).
            return Ok(base.wrapping_add(u64::from(psx.wrapping_sub(psx_base))));
        }
    } else if psx & 0xC0_0000 == 0xC0_0000 {
        let base = memory.u64(mapper + 8 + ENTRY * segment)?;
        return Ok(base.wrapping_add(u64::from(psx & 0x3F_FFFF)));
    }
    if psx.wrapping_sub(0x1F80_0000) <= 0x3FF {
        return Ok(mapper + SCRATCHPAD + u64::from(psx - 0x1F80_0000));
    }
    Ok(0)
}

/// The used window holding `host` closest to its start (below 3 MB, first
/// wins on ties).
fn closest(memory: &Memory, mapper: u64, host: u64) -> Result<Option<u64>, Fault> {
    let count = memory.u32(mapper + COUNT)? as i32;
    let mut best = None;
    let mut best_offset = WINDOW;
    for index in 0..count.max(0) as u64 {
        let entry = mapper + ENTRY * index;
        if memory.u8(entry)? == 0 {
            continue;
        }
        let (start, end) = (memory.u64(entry + 16)?, memory.u64(entry + 24)?);
        if host >= start && host < end && best_offset > host - start {
            best_offset = host - start;
            best = Some(index);
        }
    }
    Ok(best)
}

fn encode(memory: &Memory, mapper: u64, index: u64, host: u64) -> Result<u32, Fault> {
    if index == RAM_ENTRY {
        let base = memory.u64(mapper + RAM_HOST_BASE)? as u32;
        return Ok((host as u32).wrapping_sub(base) | 0x8000_0000);
    }
    let base = memory.u64(mapper + ENTRY * index + 8)? as u32;
    Ok(((index as u32) << 24) | 0xC0_0000 | (host as u32).wrapping_sub(base))
}

/// `PsxPtrToAddr`: the PSX address of `host`, or 0 when no window holds it.
pub fn host_to_psx(memory: &Memory, mapper: u64, host: u64) -> Result<u32, Fault> {
    match closest(memory, mapper, host)? {
        Some(index) => encode(memory, mapper, index, host),
        None => Ok(0),
    }
}

/// `PsxAddrMapperRegister` (and the copy inlined in `PsxClearOTagR`,
/// `signed_count` true): the PSX address of `host`, opening a window at it
/// when none holds it. 0 for a null address or a full table (254).
pub fn register(
    memory: &mut Memory,
    mapper: u64,
    host: u64,
    signed_count: bool,
) -> Result<u32, Fault> {
    if host == 0 {
        return Ok(0);
    }
    if let Some(index) = closest(memory, mapper, host)? {
        return encode(memory, mapper, index, host);
    }
    let count = memory.u32(mapper + COUNT)? as i32;
    if count >= 254 {
        return Ok(0);
    }
    let mut slot: i64 = 0;
    if count >= 0 {
        while slot <= i64::from(count) {
            if slot != RAM_ENTRY as i64 && memory.u8(mapper + ENTRY * slot as u64)? == 0 {
                break;
            }
            slot += 1;
        }
        if slot == RAM_ENTRY as i64 {
            slot = RAM_ENTRY as i64 + 1;
        }
    }
    let entry = mapper + ENTRY * slot as u64;
    memory.set_u64(entry + 8, host.wrapping_sub(0x2_0000))?;
    memory.set_u8(entry + 1, slot as u8)?;
    memory.set_u8(entry, 1)?;
    memory.set_u64(entry + 16, host)?;
    memory.set_u64(entry + 24, host.wrapping_add(WINDOW))?;
    let grown = slot as i32 + 1;
    let grow = if signed_count {
        grown > count
    } else {
        (grown as u32) > (count as u32)
    };
    if grow {
        memory.set_u32(mapper + COUNT, grown as u32)?;
    }
    encode(memory, mapper, slot as u64, host)
}

/// `Part106bSelectRamBank`'s mapper half: RAM (entry 128) maps PSX
/// `psx_base` to `host_base`, 2 MB.
pub fn select_ram(
    memory: &mut Memory,
    mapper: u64,
    psx_base: u32,
    host_base: u64,
) -> Result<(), Fault> {
    memory.set_u64(mapper + RAM_HOST_BASE, host_base)?;
    memory.set_u32(
        mapper + RAM_PSX_ADDRESS,
        (psx_base & 0x0FFF_FFFF) | 0x8000_0000,
    )?;
    let entry = mapper + ENTRY * RAM_ENTRY;
    memory.set_u8(entry, 1)?;
    memory.set_u64(entry + 8, host_base.wrapping_sub(0x2_0000))?;
    memory.set_u64(entry + 16, host_base)?;
    memory.set_u64(entry + 24, host_base.wrapping_add(0x20_0000))?;
    Ok(())
}
