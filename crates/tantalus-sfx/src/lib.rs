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
pub mod plugin;
pub mod vram;

#[cfg(feature = "dll-oracle")]
pub mod coverage;
#[cfg(feature = "dll-oracle")]
pub mod oracle;

use tantalus_assets::pc::special_effect::EffectFile;

/// What the battle view holds: the plugin's tables (read from the player's
/// DLL, never run) and the VRAM the effects draw from.
#[derive(Debug)]
pub struct SfxRuntime {
    image: image::PluginImage,
    vram: vram::Vram,
}

impl SfxRuntime {
    /// `SFX_InitSystem`: reads the plugin image and uploads its templates.
    /// `None` when `dll` is not the plugin's PE image.
    pub fn new(dll: Vec<u8>) -> Option<Self> {
        let image = image::PluginImage::parse(dll)?;
        let mut vram = vram::Vram::default();
        loader::apply(&mut vram, &loader::init_uploads(&image)?);
        Some(Self { image, vram })
    }

    /// Loads the textures and CLUTs of an `ef<id>` package into VRAM, as
    /// `SFX_Play` and the loader do, and returns the uploads made.
    pub fn load_effect(&mut self, bytes: &[u8]) -> Result<Vec<loader::Upload>, String> {
        let effect = EffectFile::parse(bytes, "effect").map_err(|error| format!("{error:?}"))?;
        let uploads = loader::effect_uploads(&effect, bytes, &self.image);
        loader::apply(&mut self.vram, &uploads);
        Ok(uploads)
    }

    pub fn vram(&self) -> &vram::Vram {
        &self.vram
    }

    pub fn image(&self) -> &image::PluginImage {
        &self.image
    }
}
