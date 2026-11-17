//! Battle special effects: a port of the PS1 effect runtime inside
//! `FF9SpecialEffectPlugin.dll` (see `docs/ff9-special-effect-plugin.md`).
//!
//! The port follows Memoria's C# rewrite of the plugin
//! (`.reference/Memoria/FF9SpecialEffectPlugin/Rewritten`, MIT, Copyright
//! (c) 2017 Albeoris) and is checked against the player's own DLL by the
//! test-only [`oracle`] (feature `dll-oracle`).

pub mod gte;
pub mod image;
pub mod interp;
pub mod loader;
pub mod mem;
pub mod mips;
pub mod player;
pub mod plugin;
pub mod render;
pub mod vram;

#[cfg(feature = "dll-oracle")]
pub mod coverage;
#[cfg(feature = "dll-oracle")]
pub mod oracle;

/// `SFX_INIT_PARAM` (300 bytes): header, eight 32-byte `BTL_DATA_INIT`
/// and the camera tail the plugin does not read.
pub const INIT_PARAM_SIZE: usize = 8 + 8 * BTL_DATA_INIT_SIZE + 36;
/// `BTL_DATA_INIT`.
pub const BTL_DATA_INIT_SIZE: usize = 32;
/// `S_Eff_Req_Org`.
pub const REQUEST_SIZE: usize = 72;

/// What the battle view holds: the plugin's image (its data read from the
/// player's DLL; the code is the translated [`plugin`], never the DLL's).
#[derive(Debug)]
pub struct SfxRuntime {
    image: image::PluginImage,
    vram: vram::Vram,
}

impl SfxRuntime {
    /// `None` when `dll` is not the plugin's PE image.
    pub fn new(dll: Vec<u8>) -> Option<Self> {
        let image = image::PluginImage::parse(dll)?;
        Some(Self {
            image,
            vram: vram::Vram::default(),
        })
    }

    /// Starts effect `id` (`ef<id>` package `bytes`) on a fresh plugin.
    pub fn play(&self, id: i32, bytes: &[u8], setup: &player::Setup) -> Option<player::EffectRun> {
        player::EffectRun::start(&self.image, id, bytes, setup)
    }

    /// Plays effect `id` to its end (at most `MAX_LOAD_FRAMES`) with one
    /// caster and one target, as `SFX_Play` and the plugin's loader
    /// (`SfxLoaderStep`) run it, and returns the `LoadImage` calls made,
    /// the init template's first. VRAM is left as the effect left it.
    pub fn load_effect(&mut self, id: i32, bytes: &[u8]) -> Result<Vec<loader::Upload>, String> {
        const MAX_LOAD_FRAMES: u32 = 2000;
        let unit = |btl_id: u16, player: bool, z: i16| player::Unit {
            btl_id,
            player,
            slot: 0,
            serial: 0,
            position: [0, 0, z],
            base: [0, 0, z],
            yaw: 0,
            height: 400,
            radius: 200,
            geo_radius: 200,
            tar_bone: 0,
        };
        let setup = player::Setup {
            units: vec![unit(1, true, 1000), unit(0x10, false, -1000)],
            caster: 0,
            targets: vec![1],
            item: false,
            args: [0; 3],
            put: [0, 0],
        };
        let mut run = self
            .play(id, bytes, &setup)
            .ok_or_else(|| format!("effect {id} did not start"))?;
        let camera = tantalus_assets::pc::battle_camera::PsxCamera {
            rotation: [[4096, 0, 0], [0, 4096, 0], [0, 0, 4096]],
            translation: [0, 0, 0],
            h: 300,
        };
        while run.frame() < MAX_LOAD_FRAMES && run.step(&camera).is_some() {}
        self.vram = run.vram();
        Ok(run.uploads())
    }

    pub fn vram(&self) -> &vram::Vram {
        &self.vram
    }

    pub fn image(&self) -> &image::PluginImage {
        &self.image
    }
}
