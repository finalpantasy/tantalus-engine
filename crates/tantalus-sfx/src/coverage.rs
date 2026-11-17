//! Test-only call coverage of the loaded plugin: an INT3 on each function
//! start records the first call, then restores the byte and resumes. Tells
//! which of the plugin's functions an effect runs, in first-call order.

use std::ffi::c_void;
use std::sync::Mutex;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualProtect(address: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
    fn AddVectoredExceptionHandler(
        first: u32,
        handler: unsafe extern "system" fn(*mut *mut c_void) -> i32,
    ) -> *mut c_void;
    fn RemoveVectoredExceptionHandler(handle: *mut c_void) -> u32;
    fn FlushInstructionCache(process: *mut c_void, address: *const c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
}

struct Armed {
    base: usize,
    original: std::collections::HashMap<usize, u8>,
    hits: Vec<u32>,
    /// RCX, RDX, R8, R9 and the fifth argument at each hit.
    args: Vec<[u64; 5]>,
}

static ARMED: Mutex<Option<Armed>> = Mutex::new(None);
static HANDLER: Mutex<usize> = Mutex::new(0);

const BREAKPOINT: u32 = 0x8000_0003;
/// `CONTEXT.Rip` on x64.
const RIP: usize = 0xF8;

unsafe extern "system" fn handler(pointers: *mut *mut c_void) -> i32 {
    // SAFETY: EXCEPTION_POINTERS is { ExceptionRecord*, ContextRecord* }.
    unsafe {
        let record = *pointers as *const u8;
        let context = *pointers.add(1) as *mut u8;
        if *(record as *const u32) != BREAKPOINT {
            return 0;
        }
        let address = *(record.add(16) as *const usize);
        let Ok(mut guard) = ARMED.try_lock() else {
            return 0;
        };
        let Some(armed) = guard.as_mut() else {
            return 0;
        };
        let Some(byte) = armed.original.remove(&address) else {
            return 0;
        };
        *(address as *mut u8) = byte;
        FlushInstructionCache(GetCurrentProcess(), address as *const c_void, 1);
        armed.hits.push((address - armed.base) as u32);
        let reg = |offset: usize| *(context.add(offset) as *const u64);
        let rsp = reg(0x98) as usize;
        armed.args.push([
            reg(0x80),
            reg(0x88),
            reg(0xB8),
            reg(0xC0),
            *((rsp + 0x28) as *const u64),
        ]);
        *(context.add(RIP) as *mut usize) = address;
        -1
    }
}

/// Arms `rvas` of the module at `base`, whose code is `text` (RVA, size).
///
/// # Safety
/// `base` must be a loaded module and every RVA a function start in it.
pub unsafe fn arm(base: usize, text: (u32, u32), rvas: &[u32]) {
    unsafe {
        let mut old = 0;
        VirtualProtect(
            (base + text.0 as usize) as *mut c_void,
            text.1 as usize,
            0x40,
            &mut old,
        );
        let mut armed = Armed {
            base,
            original: Default::default(),
            hits: Vec::new(),
            args: Vec::new(),
        };
        for &rva in rvas {
            let address = base + rva as usize;
            armed.original.insert(address, *(address as *const u8));
            *(address as *mut u8) = 0xCC;
        }
        FlushInstructionCache(GetCurrentProcess(), std::ptr::null(), 0);
        *ARMED.lock().unwrap() = Some(armed);
        let mut handle = HANDLER.lock().unwrap();
        if *handle == 0 {
            *handle = AddVectoredExceptionHandler(1, handler) as usize;
        }
    }
}

/// The RVAs hit since [`arm`], in first-call order; disarms the rest.
pub fn take() -> Vec<u32> {
    take_with_args().into_iter().map(|(rva, _)| rva).collect()
}

/// [`take`] with the first five argument words of each first call.
pub fn take_with_args() -> Vec<(u32, [u64; 5])> {
    let armed = ARMED.lock().unwrap().take();
    let Some(armed) = armed else {
        return Vec::new();
    };
    for (&address, &byte) in &armed.original {
        // SAFETY: restoring the bytes `arm` replaced.
        unsafe { *(address as *mut u8) = byte };
    }
    let mut handle = HANDLER.lock().unwrap();
    if *handle != 0 {
        // SAFETY: the handle came from AddVectoredExceptionHandler.
        unsafe { RemoveVectoredExceptionHandler(*handle as *mut c_void) };
        *handle = 0;
    }
    armed.hits.into_iter().zip(armed.args).collect()
}
