//! The plugin itself, translated from Memoria's C# rewrite
//! (`.reference/Memoria/FF9SpecialEffectPlugin/Rewritten`, MIT, Copyright
//! (c) 2017 Albeoris) by `cs2rs` over the flat memory model of [`crate::mem`].
//! `generated.rs` is that tool's output; [`rt`] is what it stands on.

#[rustfmt::skip]
pub mod generated;
pub mod debug_host;
pub mod rt;

pub use rt::{HOST_CALLBACK, Host, Plugin};

use crate::image::PluginImage;
use crate::mem::Memory;

/// One primitive of a frame: its ordering-table depth and its packet words
/// (the `P_TAG` header first).
pub type Prim = (i32, Vec<u32>);

impl Plugin {
    /// The DLL's state right after `LoadLibrary`: the module image with its
    /// data sections, and the CRT initializers run
    /// (`ModuleInitializers.g.cs`). `None` when `dll` has no sections.
    pub fn load(dll: &PluginImage, host: Box<dyn Host>) -> Option<Self> {
        let memory = Memory::from_image(dll)?;
        let mut plugin = Self::with_memory(memory, host);
        plugin.InitMapperTable();
        Some(plugin)
    }

    /// Copies `bytes` to a fresh heap block and returns its address.
    pub fn put(&mut self, bytes: &[u8]) -> u64 {
        let at = self.m.alloc(bytes.len());
        self.m.write_bytes(at, bytes).expect("heap block");
        at
    }

    /// `SFX_InitSystem(callback)`.
    pub fn init_system(&mut self) {
        self.SFX_InitSystem(HOST_CALLBACK);
    }

    /// `SFX_InitBattle(param)` with a full `SFX_INIT_PARAM`.
    pub fn init_battle(&mut self, param: &[u8]) {
        let at = self.put(param);
        self.SFX_InitBattle_0(at);
    }

    /// `SFX_Play(effnum, bin, size, req)`.
    pub fn play(&mut self, effect: i32, bytes: &[u8], request: &[u8]) {
        let bin = self.put(bytes);
        let req = self.put(request);
        self.SFX_Play_0(effect, bin, bytes.len() as i32, req);
    }

    /// One frame as the host runs it (`SFX.cs:591-653`): `SFX_Update`,
    /// `SFX_LateUpdate`, then the frame's primitives from `SFX_BeginRender`
    /// and `SFX_GetPrim`. `None` once the effect has ended.
    pub fn frame(&mut self) -> Option<Vec<Prim>> {
        let index = self.put(&[0; 4]);
        let running = self.SFX_Update_0(index) != 0;
        self.SFX_LateUpdate();
        if !running {
            return None;
        }
        let mut prims = Vec::new();
        if self.SFX_BeginRender_0() != 0 {
            let depth = self.put(&[0; 4]);
            for _ in 0..100_000 {
                let tag = self.SFX_GetPrim_0(depth);
                if tag == 0 {
                    break;
                }
                let header = self.ld::<u32>(tag);
                let words = (0..=u64::from(header >> 24))
                    .map(|i| self.ld::<u32>(tag + 4 * i))
                    .collect();
                prims.push((self.ld::<i32>(depth), words));
            }
        }
        Some(prims)
    }
}
