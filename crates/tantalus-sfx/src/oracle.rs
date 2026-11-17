//! Test-only harness that runs the player's own `FF9SpecialEffectPlugin.dll`
//! so the port can be compared with it frame by frame. Built only with the
//! `dll-oracle` feature; it is never a runtime fallback (STATE.md) and
//! nothing in the engine depends on it.
//!
//! The host side follows Memoria's `SFX.DebugRoomCallback` (`SFX.cs:1401`):
//! VRAM calls go to a [`Vram`], unit queries answer with one fixed position
//! and identity transforms, everything else returns 0. Every callback is
//! logged.

use crate::vram::{Rect, Vram};
use std::sync::Mutex;

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

#[derive(Default)]
struct HostState {
    vram: Vram,
    calls: Vec<Call>,
    frame: u32,
}

static HOST: Mutex<Option<HostState>> = Mutex::new(None);
/// The plugin keeps global state: one oracle at a time.
static IN_USE: Mutex<()> = Mutex::new(());

/// FNV-1a over the little-endian bytes of `words`.
pub fn fnv(words: &[u16]) -> u64 {
    words.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, word| {
        word.to_le_bytes().iter().fold(hash, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        })
    })
}

/// Where the harness puts battle object `btl_id`: party members in front
/// of the camera, enemies behind them.
fn position(btl_id: u32) -> [i16; 3] {
    let slot = btl_id.trailing_zeros() as i16;
    if btl_id >= 0x10 {
        [(slot - 4) * 400, 0, -1000]
    } else {
        [slot * 400 - 600, 0, 1000]
    }
}

fn log(
    host: &mut HostState,
    command: u8,
    code: i32,
    args: [i32; 4],
    pixels: Option<u64>,
    result: i32,
) -> i32 {
    host.calls.push(Call {
        frame: host.frame,
        command,
        low: code as u32 & 0xFF_FFFF,
        args,
        pixels,
    });
    result
}

/// `SFX.Callback` (`SFX.cs:2537`): `Int32 (Int32 code, Int32, Int32, Int32,
/// Int32, void* p)`.
unsafe extern "C" fn callback(code: i32, a0: i32, a1: i32, a2: i32, a3: i32, p: *mut u8) -> i32 {
    let mut guard = HOST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(host) = guard.as_mut() else {
        return 0;
    };
    let command = (code as u32 >> 24) as u8;
    let rect = Rect {
        x: a0 as i16,
        y: a1 as i16,
        w: a2 as i16,
        h: a3 as i16,
    };
    let mut pixels = None;
    match command {
        100 if !p.is_null() => {
            let count = (i32::from(rect.w).max(0) * i32::from(rect.h).max(0)) as usize;
            // SAFETY: the plugin passes `w * h` pixels for `LoadImage`.
            let words = unsafe { std::slice::from_raw_parts(p.cast::<u16>(), count) };
            pixels = Some(fnv(words));
            host.vram.load_image(rect, words);
        }
        101 if !p.is_null() => {
            let words = host.vram.store_image(rect);
            // SAFETY: `StoreImage` gives a buffer for `w * h` pixels.
            unsafe { std::ptr::copy_nonoverlapping(words.as_ptr(), p.cast::<u16>(), words.len()) };
        }
        102 if !p.is_null() => {
            // SAFETY: `MoveImage` passes the source RECT as four i16.
            let source = unsafe { std::slice::from_raw_parts(p.cast::<i16>(), 4) };
            host.vram.move_image(
                Rect {
                    x: source[0],
                    y: source[1],
                    w: source[2],
                    h: source[3],
                },
                a0 as i16,
                a1 as i16,
            );
        }
        // GetPosition (`DebugRoomCallback` case 1): the object's SVECTOR,
        // modes 2 and 3 only x and z.
        1 if !p.is_null() => {
            let at = position(code as u32 & 0xFF);
            let words: [i16; 3] = if a0 >= 2 { [at[0], 0, at[2]] } else { at };
            // SAFETY: an SVECTOR (3 i16).
            unsafe { std::ptr::copy_nonoverlapping(words.as_ptr().cast::<u8>(), p, 6) };
        }
        // GetRotate: zero angles.
        3 if !p.is_null() => {
            // SAFETY: an SVECTOR (3 i16).
            unsafe { std::ptr::write_bytes(p, 0, 6) };
        }
        // GetScale: three i32 of 4096.
        5 if !p.is_null() => {
            let scale: [i32; 3] = [4096; 3];
            // SAFETY: three i32.
            unsafe { std::ptr::copy_nonoverlapping(scale.as_ptr().cast::<u8>(), p, 12) };
        }
        // GetMotionFrameMax and IsTarget answer 1.
        9 | 15 => return log(host, command, code, [a0, a1, a2, a3], None, 1),
        // GetMatrix: mode 0 writes the position, mode 1 returns its y.
        14 => {
            let at = position(code as u32 & 0xFF);
            if a0 == 1 {
                return log(
                    host,
                    command,
                    code,
                    [a0, a1, a2, a3],
                    None,
                    i32::from(at[1]),
                );
            }
            if !p.is_null() {
                // SAFETY: an SVECTOR (3 i16).
                unsafe { std::ptr::copy_nonoverlapping(at.as_ptr().cast::<u8>(), p, 6) };
            }
        }
        _ => {}
    }
    log(host, command, code, [a0, a1, a2, a3], pixels, 0)
}

type InitSystem =
    unsafe extern "C" fn(unsafe extern "C" fn(i32, i32, i32, i32, i32, *mut u8) -> i32);
type InitBattle = unsafe extern "C" fn(*const u8);
type Play = unsafe extern "C" fn(i32, *const u8, i32, *const u8);
type Update = unsafe extern "C" fn(*mut i32) -> u8;
type Void = unsafe extern "C" fn();
type BeginRender = unsafe extern "C" fn() -> u8;
type GetPrim = unsafe extern "C" fn(*mut i32) -> *const u8;

/// One primitive of a frame: its ordering-table depth and its packet words
/// (the `P_TAG` header first).
pub type Prim = (i32, Vec<u32>);

/// The loaded plugin. Dropping it unloads the DLL.
pub struct Oracle {
    library: libloading::Library,
    _lock: std::sync::MutexGuard<'static, ()>,
    frame: i32,
}

/// `SFX_INIT_PARAM` (`SFX_INIT_PARAM.cs`): 8 header bytes, eight 36-byte
/// `BTL_DATA_INIT`, `camMat[9]`, `camTrans[3]`, `camProj`, pad.
pub const INIT_PARAM_SIZE: usize = 8 + 8 * 36 + 18 + 12 + 4;
/// `S_Eff_Req_Org` (`S_Eff_Req_Org.cs`): 14 `BTL_DATA_REQ` then 16 bytes.
pub const REQUEST_SIZE: usize = 72;

impl Oracle {
    /// Loads `dll` (with its directory on the search path for its CRT) and
    /// runs `SFX_InitSystem`.
    pub fn load(dll: &std::path::Path) -> Result<Self, String> {
        let lock = IN_USE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *HOST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(HostState::default());
        // SAFETY: test-only; the player's own plugin, loaded with the
        // altered search path so its msvcr120 next to it resolves.
        let library = unsafe {
            libloading::os::windows::Library::load_with_flags(
                dll,
                libloading::os::windows::LOAD_WITH_ALTERED_SEARCH_PATH,
            )
        }
        .map_err(|error| error.to_string())?;
        let oracle = Self {
            library: library.into(),
            _lock: lock,
            frame: 0,
        };
        // SAFETY: the export's signature is Memoria's DllImport.
        unsafe { oracle.symbol::<InitSystem>(b"SFX_InitSystem")?(callback) };
        Ok(oracle)
    }

    fn symbol<T: Copy>(&self, name: &[u8]) -> Result<T, String> {
        // SAFETY: `T` is the export's function pointer type.
        unsafe {
            self.library
                .get::<T>(name)
                .map(|symbol| *symbol)
                .map_err(|e| e.to_string())
        }
    }

    /// `SFX_InitBattle` with `param` (see [`INIT_PARAM_SIZE`]).
    pub fn init_battle(&mut self, param: &[u8; INIT_PARAM_SIZE]) -> Result<(), String> {
        // SAFETY: `param` is a full SFX_INIT_PARAM.
        unsafe { self.symbol::<InitBattle>(b"SFX_InitBattle")?(param.as_ptr()) };
        Ok(())
    }

    /// `SFX_Play(effnum, bin, size, req)`.
    pub fn play(
        &mut self,
        effect: i32,
        bytes: &[u8],
        request: &[u8; REQUEST_SIZE],
    ) -> Result<(), String> {
        let size = i32::try_from(bytes.len()).map_err(|e| e.to_string())?;
        self.frame = 0;
        // SAFETY: the plugin copies `size` bytes and reads a full request.
        unsafe {
            self.symbol::<Play>(b"SFX_Play")?(effect, bytes.as_ptr(), size, request.as_ptr())
        };
        Ok(())
    }

    /// One frame: `SFX_Update`, `SFX_LateUpdate`, then the primitives of
    /// the frame (`SFX_BeginRender` and `SFX_GetPrim` until null), each as
    /// (depth, the packet's words). `None` once the effect has ended.
    pub fn frame(&mut self) -> Result<Option<Vec<Prim>>, String> {
        if let Some(host) = HOST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            host.frame += 1;
        }
        // SAFETY: Memoria's signatures.
        let running = unsafe { self.symbol::<Update>(b"SFX_Update")?(&mut self.frame) } != 0;
        unsafe { self.symbol::<Void>(b"SFX_LateUpdate")?() };
        if !running {
            return Ok(None);
        }
        let mut prims = Vec::new();
        if unsafe { self.symbol::<BeginRender>(b"SFX_BeginRender")?() } != 0 {
            let get = self.symbol::<GetPrim>(b"SFX_GetPrim")?;
            for _ in 0..100_000 {
                let mut depth = 0;
                // SAFETY: returns a P_TAG pointer or null.
                let tag = unsafe { get(&mut depth) };
                if tag.is_null() {
                    break;
                }
                // P_TAG: u32 addr:24 len:8, then `len` words.
                let header = unsafe { tag.cast::<u32>().read_unaligned() };
                let len = (header >> 24) as usize;
                let words = (0..=len)
                    .map(|i| unsafe { tag.cast::<u32>().add(i).read_unaligned() })
                    .collect();
                prims.push((depth, words));
            }
        }
        Ok(Some(prims))
    }

    /// The calls logged so far, cleared.
    pub fn take_calls(&mut self) -> Vec<Call> {
        HOST.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
            .map(|host| std::mem::take(&mut host.calls))
            .unwrap_or_default()
    }

    /// The host VRAM as the plugin left it.
    pub fn vram(&self) -> Vram {
        HOST.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|host| host.vram.clone())
            .unwrap_or_default()
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        *HOST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}
