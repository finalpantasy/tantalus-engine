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

use crate::plugin::debug_host::position;
pub use crate::plugin::debug_host::{Call, fnv};

#[derive(Default)]
struct HostState {
    vram: Vram,
    calls: Vec<Call>,
    frame: u32,
    /// Answers as the battle view's host does, when set.
    battle: Option<crate::player::BattleHost>,
}

/// The process's own memory, for [`crate::player::BattleHost::answer`].
struct Native;

impl crate::player::Access for Native {
    fn read(&self, at: u64, len: usize) -> Option<Vec<u8>> {
        // SAFETY: test-only; `at` is a pointer the DLL handed its callback.
        (at != 0).then(|| unsafe { std::slice::from_raw_parts(at as *const u8, len) }.to_vec())
    }

    fn write(&mut self, at: u64, bytes: &[u8]) {
        if at != 0 {
            // SAFETY: as above, a buffer the DLL passed to be filled.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), at as *mut u8, bytes.len()) };
        }
    }
}

static HOST: Mutex<Option<HostState>> = Mutex::new(None);
/// The plugin keeps global state: one oracle at a time.
static IN_USE: Mutex<()> = Mutex::new(());

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
    if let Some(battle) = host.battle.as_mut() {
        battle.frame = host.frame;
        return battle.answer(&mut Native, code, [a0, a1, a2, a3], p as u64);
    }
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
        // GetMatrix: mode 0 writes the position, mode 1 returns its y, mode 2 a
        // whole `PSXMAT` (see `debug_host::matrix`).
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
            if !p.is_null() && a0 == 2 {
                let matrix = crate::plugin::debug_host::matrix(code as u32 & 0xFF);
                // SAFETY: mode 2 passes a PSX MATRIX (32 bytes).
                unsafe { std::ptr::copy_nonoverlapping(matrix.as_ptr(), p, 32) };
            } else if !p.is_null() {
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

/// A script as the plugin decoded it (see [`Oracle::decoded_scripts`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedScript {
    pub bank: usize,
    pub address: u32,
    pub words: Vec<u32>,
    pub records: Vec<[u32; 4]>,
}

/// The loaded plugin. Dropping it unloads the DLL.
pub struct Oracle {
    library: libloading::Library,
    /// The module's load address (its `HMODULE`).
    base: usize,
    _lock: std::sync::MutexGuard<'static, ()>,
    frame: i32,
}

pub use crate::{INIT_PARAM_SIZE, REQUEST_SIZE};

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
        let raw = library.into_raw();
        let base = raw as usize;
        // SAFETY: `raw` came from `into_raw` just above.
        let library = unsafe { libloading::os::windows::Library::from_raw(raw) };
        let oracle = Self {
            library: library.into(),
            base,
            _lock: lock,
            frame: 0,
        };
        // SAFETY: the export's signature is Memoria's DllImport.
        unsafe { oracle.symbol::<InitSystem>(b"SFX_InitSystem")?(callback) };
        Ok(oracle)
    }

    /// `len` bytes of the loaded module at `rva`.
    pub fn module_bytes(&self, rva: u32, len: usize) -> Vec<u8> {
        // SAFETY: test-only; callers pass ranges inside the mapped image.
        unsafe { std::slice::from_raw_parts((self.base + rva as usize) as *const u8, len).to_vec() }
    }

    /// Writes `bytes` into the loaded module at `rva` (test-only: GTE
    /// registers and similar `.data` globals).
    pub fn write_module_bytes(&mut self, rva: u32, bytes: &[u8]) {
        // SAFETY: test-only; callers pass ranges inside the module's
        // writable data.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                (self.base + rva as usize) as *mut u8,
                bytes.len(),
            );
        }
    }

    /// The module's internal function at `rva`, as a pointer to cast to
    /// its signature.
    pub fn internal(&self, rva: u32) -> *const () {
        (self.base + rva as usize) as *const ()
    }

    /// The module's load address.
    pub fn base(&self) -> usize {
        self.base
    }

    /// The scripts the plugin decoded last, per bank: the PSX address, the
    /// MIPS words and the 16-byte records `MipsDecodeAll` made of them (the
    /// thread states at `qword_1803678E0 + 3096 + 64 * bank`,
    /// `SfxLoaderStep.cs:413-425`).
    pub fn decoded_scripts(&self) -> Vec<DecodedScript> {
        let mut out = Vec::new();
        // SAFETY: test-only reads of the plugin's own heap through the
        // pointers it keeps; null pointers are skipped.
        unsafe {
            let context = *((self.base + 0x3678E0) as *const usize);
            if context == 0 {
                return out;
            }
            for bank in 0..2 {
                let thread = context + 3096 + 64 * bank;
                let address = *(thread as *const u32);
                let size = *((thread + 4) as *const u32) as usize;
                let code = *((thread + 8) as *const usize);
                let records = *((thread + 24) as *const usize);
                if code == 0 || records == 0 || size == 0 || size > 1 << 20 {
                    continue;
                }
                let words = std::slice::from_raw_parts(code as *const u32, size / 4).to_vec();
                let records =
                    std::slice::from_raw_parts(records as *const [u32; 4], size / 4).to_vec();
                out.push(DecodedScript {
                    bank,
                    address,
                    words,
                    records,
                });
            }
        }
        out
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

    /// Answers callbacks as the battle view's host does, over `units`.
    pub fn use_battle_host(&mut self, units: &[crate::player::Unit]) {
        if let Some(host) = HOST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            // `SFX_InitSystem`'s uploads went to the plain host.
            host.battle = Some(crate::player::BattleHost {
                vram: host.vram.clone(),
                units: units.to_vec(),
                intensity: 128,
                ..Default::default()
            });
        }
    }

    /// Moves the battle host's units.
    pub fn set_units(&mut self, units: &[crate::player::Unit]) {
        if let Some(battle) = HOST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
            .and_then(|host| host.battle.as_mut())
        {
            battle.units = units.to_vec();
        }
    }

    /// A frame's packets drawn as the player draws them, over the battle
    /// host's VRAM (`None` without the battle host).
    pub fn render(&self, effect: i32, prims: &[Prim]) -> Option<crate::render::RenderList> {
        HOST.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
            .and_then(|host| host.battle.as_mut())
            .map(|battle| crate::render::render_list(effect, prims, &mut battle.vram))
    }

    /// Writes `camera` where `SFX_Update` reads it, as the player does.
    pub fn set_camera(&mut self, camera: &tantalus_assets::pc::battle_camera::PsxCamera) {
        const IMAGE_BASE: u64 = 0x1_8000_0000;
        let (matrix, distance) = crate::player::camera_bytes(camera);
        self.write_module_bytes((crate::player::CAMERA_MATRIX - IMAGE_BASE) as u32, &matrix);
        self.write_module_bytes(
            (crate::player::CAMERA_DISTANCE - IMAGE_BASE) as u32,
            &distance,
        );
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
