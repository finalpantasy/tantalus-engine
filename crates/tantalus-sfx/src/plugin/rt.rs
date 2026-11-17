//! What the generated translation stands on: the plugin state, typed loads
//! and stores over [`Memory`], the native stack, and the few runtime
//! services Memoria's rewrite takes from .NET (`Runtime/X86.cs`, `Crt.cs`,
//! `Ports.cs`).

pub use crate::mem::Memory;
use crate::mem::{STACK_BASE, STACK_SIZE};

/// The value `SFX_InitSystem` stores as the host callback pointer; calls
/// through it go to [`Host::callback`].
pub const HOST_CALLBACK: u64 = 0xFFFF_0000_0000_0001;

/// Unity's side of the plugin: `SFX.Callback` (`SFX.cs:2537`), `Int32 (Int32
/// code, Int32, Int32, Int32, Int32, void* p)`. `p` is an address in `memory`.
pub trait Host: Send {
    #[allow(clippy::too_many_arguments)]
    fn callback(
        &mut self,
        memory: &mut Memory,
        code: i32,
        a0: i32,
        a1: i32,
        a2: i32,
        a3: i32,
        p: u64,
    ) -> i32;
}

/// A `Span<T>`: its first element's address and its length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sp(pub u64, pub i32);

/// Pointer arithmetic: `p + i` for elements of `size` bytes.
#[inline]
pub fn pa(p: u64, i: i64, size: u64) -> u64 {
    p.wrapping_add((i as u64).wrapping_mul(size))
}

/// `X86.Cvttsd2si`: truncation, with the x86 "integer indefinite" for NaN
/// and out-of-range values.
#[inline]
pub fn cvttsd2si(value: f64) -> i32 {
    if !(-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}

/// A plain value the translation loads and stores.
pub trait Scalar: Copy {
    const SIZE: usize;
    fn read(bytes: &[u8]) -> Self;
    fn write(self, bytes: &mut [u8]);
}

macro_rules! scalar {
    ($($t:ty),*) => {$(
        impl Scalar for $t {
            const SIZE: usize = std::mem::size_of::<$t>();
            #[inline]
            fn read(bytes: &[u8]) -> Self {
                <$t>::from_le_bytes(bytes.try_into().unwrap())
            }
            #[inline]
            fn write(self, bytes: &mut [u8]) {
                bytes.copy_from_slice(&self.to_le_bytes());
            }
        }
    )*};
}
scalar!(i8, u8, i16, u16, i32, u32, i64, u64, i128, u128, f32, f64);

impl Scalar for bool {
    const SIZE: usize = 1;
    fn read(bytes: &[u8]) -> Self {
        bytes[0] != 0
    }
    fn write(self, bytes: &mut [u8]) {
        bytes[0] = u8::from(self);
    }
}

/// The plugin: its memory (module image, heap, stack) and the host.
pub struct Plugin {
    pub m: Memory,
    pub host: Box<dyn Host>,
    pub statics: super::generated::Statics,
    /// The native stack pointer (grows down to [`STACK_BASE`]).
    pub sp: u64,
    consts: std::collections::HashMap<u32, u64>,
    /// What `Ports.Console` printed.
    pub console_log: String,
    /// Where the original module is taken to be loaded, for the few places
    /// it cuts a module address to 32 bits ([`Plugin::low32`]). The DLL's
    /// preferred base by default; tests set the oracle's load address.
    pub module_base: u64,
    /// Debugging: when set, every translated function records its name in
    /// [`Plugin::calls`] (the last 4096).
    pub tracing: bool,
    pub calls: std::collections::VecDeque<&'static str>,
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugin").field("m", &self.m).finish()
    }
}

impl Plugin {
    /// A plugin over `memory` (the module image), before any initializer.
    pub fn with_memory(m: Memory, host: Box<dyn Host>) -> Self {
        Self {
            m,
            host,
            statics: Default::default(),
            // Headroom above the first frame: scripts keep PSX addresses of
            // stack locals and write through them after the frame is gone
            // (the original scribbles on its caller's stack there).
            sp: STACK_BASE + STACK_SIZE as u64 - (1 << 20),
            consts: Default::default(),
            console_log: String::new(),
            module_base: crate::mem::IMAGE_BASE,
            tracing: false,
            calls: Default::default(),
        }
    }

    #[inline]
    #[track_caller]
    pub fn ld<T: Scalar>(&self, address: u64) -> T {
        match self.m.bytes(address, T::SIZE) {
            Ok(bytes) => T::read(bytes),
            Err(fault) => panic!("read fault at {:#x}", fault.0),
        }
    }

    #[inline]
    #[track_caller]
    pub fn st<T: Scalar>(&mut self, address: u64, value: T) {
        let mut bytes = [0u8; 16];
        value.write(&mut bytes[..T::SIZE]);
        if let Err(fault) = self.m.write_bytes(address, &bytes[..T::SIZE]) {
            panic!("write fault at {:#x}", fault.0);
        }
    }

    pub fn trace(&mut self, name: &'static str) {
        if self.calls.len() >= 4096 {
            self.calls.pop_front();
        }
        self.calls.push_back(name);
    }

    /// A pointer cut to 32 bits as the original computes it: module
    /// addresses relative to [`Plugin::module_base`] (RTPS multiplies by the
    /// DQA register's address; the loader subtracts table addresses).
    pub fn low32(&self, address: u64) -> u32 {
        let image = crate::mem::IMAGE_BASE;
        if (image..image + 0x1000_0000).contains(&address) {
            (address - image + self.module_base) as u32
        } else {
            address as u32
        }
    }

    /// Zeroed native stack space, 16-byte aligned; freed when the
    /// translated function that asked for it returns. The stack grows down
    /// like the original's, so a deeper frame's locals sit below its
    /// caller's: the PSX address mapper opens a window for a stack address
    /// below every open one, and the port must open the same windows.
    pub fn salloc(&mut self, size: usize) -> u64 {
        let at = (self.sp - size.max(1) as u64) & !15;
        assert!(at >= STACK_BASE, "native stack overflow");
        self.sp = at;
        self.m.fill(at, 0, size);
        at
    }

    /// Constant data (C# `u8` literals and constant spans), placed once.
    pub fn const_data(&mut self, id: u32, bytes: &[u8]) -> u64 {
        if let Some(&at) = self.consts.get(&id) {
            return at;
        }
        let at = self.m.alloc(bytes.len());
        self.m.write_bytes(at, bytes).expect("const data");
        self.consts.insert(id, at);
        at
    }

    pub fn host_callback(&mut self, code: i32, a0: i32, a1: i32, a2: i32, a3: i32, p: u64) -> i32 {
        self.host.callback(&mut self.m, code, a0, a1, a2, a3, p)
    }

    pub fn fail_fast(&mut self, message: String) {
        panic!("plugin assertion: {message}");
    }

    pub fn bad_call(&mut self, address: u64) -> i64 {
        panic!("call to {address:#x}: no translated function there");
    }

    pub fn console(&mut self, text: String) {
        self.console_log.push_str(&text);
    }

    /// `srand`: the CRT's generator, which no translated code reads.
    pub fn crt_srand(&mut self, _seed: u32) {}

    /// `Ports.Clock.UtcNow`: fixed, so runs repeat.
    pub fn unix_time(&mut self) -> i64 {
        0
    }
}
