//! Background sprites with blend modes 1-3, as Memoria's
//! `PSX/FieldMap_Abr_1`..`_3` draw them: mode 1 adds the sprite, mode 2
//! subtracts it (`BlendOp RevSub`), and mode 3 adds it too. The PSX
//! quarter weight of mode 3 does not survive there: its shader writes 0.25
//! only to alpha, which `Blend One One` ignores. Mode 0 is plain alpha
//! blending and stays a `Sprite`.

use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin};

/// Registers the shader and the material.
pub struct PsxBlendPlugin;

impl Plugin for PsxBlendPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/psx_blend.wgsl");
        embedded_asset!(app, "shaders/fade_blend.wgsl");
        app.add_plugins(Material2dPlugin::<PsxBlendMaterial>::default());
        app.add_plugins(Material2dPlugin::<FadeBlendMaterial>::default());
    }
}

#[derive(Debug, Clone, Copy, Default, ShaderType)]
pub struct PsxBlendUniform {
    pub rect: Vec4,
    pub kill_below: f32,
}

/// One blended sprite: its atlas, the normalized atlas rectangle, and the
/// PSX mode (1, 2 or 3).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(PsxBlendKey)]
pub struct PsxBlendMaterial {
    #[uniform(0)]
    pub uniform: PsxBlendUniform,
    #[texture(1)]
    #[sampler(2)]
    pub atlas: Handle<Image>,
    pub mode: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PsxBlendKey {
    mode: u8,
}

impl From<&PsxBlendMaterial> for PsxBlendKey {
    fn from(material: &PsxBlendMaterial) -> Self {
        Self {
            mode: material.mode,
        }
    }
}

impl PsxBlendMaterial {
    /// `rect` is the atlas rectangle in pixels (x0, y0, x1, y1) of an atlas
    /// `size` pixels across. `None` for a mode that is not 1, 2 or 3.
    pub fn new(atlas: Handle<Image>, rect: [f32; 4], size: [f32; 2], mode: u8) -> Option<Self> {
        if !(1..=3).contains(&mode) || size[0] <= 0.0 || size[1] <= 0.0 {
            return None;
        }
        Some(Self {
            uniform: PsxBlendUniform {
                rect: Vec4::new(
                    rect[0] / size[0],
                    rect[1] / size[1],
                    rect[2] / size[0],
                    rect[3] / size[1],
                ),
                // `texkill` thresholds: 0.01 in `_Abr_1`, 0.1 in the others.
                kill_below: if mode == 1 { 0.01 } else { 0.1 },
            },
            atlas,
            mode,
        })
    }
}

/// The blend state for a mode: modes 1 and 3 add the sprite, mode 2
/// subtracts it from what is already drawn.
pub fn blend_state(mode: u8) -> BlendState {
    let operation = if mode == 2 {
        BlendOperation::ReverseSubtract
    } else {
        BlendOperation::Add
    };
    let component = BlendComponent {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::One,
        operation,
    };
    BlendState {
        color: component,
        alpha: BlendComponent {
            src_factor: BlendFactor::Zero,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        },
    }
}

impl Material2d for PsxBlendMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("shaders/psx_blend.wgsl"))
                .with_source("embedded"),
        )
    }

    // Sorted with the sprites by z in the transparent phase.
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(blend_state(key.bind_group_data.mode));
            }
        }
        Ok(())
    }
}

/// The script fade filter as a full-screen quad: its color is added to (mode 0) or
/// subtracted from (mode 2) what is already drawn, per channel.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(PsxBlendKey)]
pub struct FadeBlendMaterial {
    #[uniform(0)]
    pub color: Vec4,
    pub mode: u8,
}

impl From<&FadeBlendMaterial> for PsxBlendKey {
    fn from(material: &FadeBlendMaterial) -> Self {
        Self {
            mode: material.mode,
        }
    }
}

impl Material2d for FadeBlendMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("shaders/fade_blend.wgsl"))
                .with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(blend_state(key.bind_group_data.mode));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_map_to_psx_arithmetic() {
        assert_eq!(blend_state(1).color.operation, BlendOperation::Add);
        assert_eq!(
            blend_state(2).color.operation,
            BlendOperation::ReverseSubtract
        );
        assert_eq!(blend_state(3), blend_state(1));
        let third =
            PsxBlendMaterial::new(Handle::default(), [0.0, 0.0, 16.0, 8.0], [32.0, 16.0], 3)
                .unwrap();
        assert_eq!(third.uniform.kill_below, 0.1);
        assert_eq!(third.uniform.rect, Vec4::new(0.0, 0.0, 0.5, 0.5));
        assert!(PsxBlendMaterial::new(Handle::default(), [0.0; 4], [1.0, 1.0], 0).is_none());
    }
}
