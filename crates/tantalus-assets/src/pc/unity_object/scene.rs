//! Unity 5.2 hierarchy and renderer objects decoded from their type trees:
//! `GameObject` (1), `Transform` (4), `Material` (21), `Animation` (111) and
//! `SkinnedMeshRenderer` (137).

use super::fields::{Aabb, Fields, PPtr};
use super::type_tree::Value;
use crate::install::ImportError;
use serde::{Deserialize, Serialize};

pub const GAME_OBJECT_CLASS_ID: u16 = 1;
pub const TRANSFORM_CLASS_ID: u16 = 4;
pub const MATERIAL_CLASS_ID: u16 = 21;
pub const ANIMATION_CLASS_ID: u16 = 111;
pub const SKINNED_MESH_RENDERER_CLASS_ID: u16 = 137;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameObject {
    pub name: String,
    pub layer: u32,
    pub tag: u32,
    pub active: bool,
    /// `(class_id, component)` pairs.
    pub components: Vec<(i32, PPtr)>,
}

impl GameObject {
    pub fn from_value(value: &Value<'_>, source: &str) -> Result<Self, ImportError> {
        let fields = Fields::new(value, source, "GameObject");
        let mut components = Vec::new();
        for pair in fields.array("m_Component")? {
            let pair = Fields::new(pair, source, "GameObject component");
            components.push((pair.i32("first")?, pair.pptr("second")?));
        }
        Ok(Self {
            name: fields.string("m_Name")?,
            layer: fields.u32("m_Layer")?,
            tag: fields.u32("m_Tag")?,
            active: fields.bool("m_IsActive")?,
            components,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub game_object: PPtr,
    pub local_rotation: [f32; 4],
    pub local_position: [f32; 3],
    pub local_scale: [f32; 3],
    pub children: Vec<PPtr>,
    pub father: PPtr,
}

impl Transform {
    pub fn from_value(value: &Value<'_>, source: &str) -> Result<Self, ImportError> {
        let fields = Fields::new(value, source, "Transform");
        Ok(Self {
            game_object: fields.pptr("m_GameObject")?,
            local_rotation: fields.quat("m_LocalRotation")?,
            local_position: fields.vec3("m_LocalPosition")?,
            local_scale: fields.vec3("m_LocalScale")?,
            children: fields.pptrs("m_Children")?,
            father: fields.pptr("m_Father")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinnedMeshRenderer {
    pub game_object: PPtr,
    pub enabled: bool,
    pub materials: Vec<PPtr>,
    pub mesh: PPtr,
    pub bones: Vec<PPtr>,
    pub root_bone: PPtr,
    pub aabb: Aabb,
    pub blend_shape_weights: Vec<f32>,
    pub update_when_offscreen: bool,
}

impl SkinnedMeshRenderer {
    pub fn from_value(value: &Value<'_>, source: &str) -> Result<Self, ImportError> {
        let fields = Fields::new(value, source, "SkinnedMeshRenderer");
        Ok(Self {
            game_object: fields.pptr("m_GameObject")?,
            enabled: fields.bool("m_Enabled")?,
            materials: fields.pptrs("m_Materials")?,
            mesh: fields.pptr("m_Mesh")?,
            bones: fields.pptrs("m_Bones")?,
            root_bone: fields.pptr("m_RootBone")?,
            aabb: fields.aabb("m_AABB")?,
            blend_shape_weights: fields.f32s("m_BlendShapeWeights")?,
            update_when_offscreen: fields.bool("m_UpdateWhenOffscreen")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    pub shader: PPtr,
    pub shader_keywords: String,
    /// `(property, texture)` pairs from `m_TexEnvs`.
    pub textures: Vec<(String, PPtr)>,
    pub floats: Vec<(String, f32)>,
    pub colors: Vec<(String, [f32; 4])>,
}

impl Material {
    pub fn from_value(value: &Value<'_>, source: &str) -> Result<Self, ImportError> {
        let fields = Fields::new(value, source, "Material");
        let properties = fields.sub("m_SavedProperties")?;
        let property_name = |pair: &Fields<'_, '_>| pair.sub("first")?.string("name");
        let mut textures = Vec::new();
        for pair in properties.array("m_TexEnvs")? {
            let pair = Fields::new(pair, source, "Material texture");
            textures.push((
                property_name(&pair)?,
                pair.sub("second")?.pptr("m_Texture")?,
            ));
        }
        let mut floats = Vec::new();
        for pair in properties.array("m_Floats")? {
            let pair = Fields::new(pair, source, "Material float");
            floats.push((property_name(&pair)?, pair.f32("second")?));
        }
        let mut colors = Vec::new();
        for pair in properties.array("m_Colors")? {
            let pair = Fields::new(pair, source, "Material color");
            colors.push((
                property_name(&pair)?,
                pair.floats("second", ["r", "g", "b", "a"])?,
            ));
        }
        Ok(Self {
            name: fields.string("m_Name")?,
            shader: fields.pptr("m_Shader")?,
            shader_keywords: fields.string("m_ShaderKeywords")?,
            textures,
            floats,
            colors,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimationComponent {
    pub game_object: PPtr,
    pub clip: PPtr,
    pub clips: Vec<PPtr>,
    pub wrap_mode: i32,
    pub play_automatically: bool,
}

impl AnimationComponent {
    pub fn from_value(value: &Value<'_>, source: &str) -> Result<Self, ImportError> {
        let fields = Fields::new(value, source, "Animation");
        Ok(Self {
            game_object: fields.pptr("m_GameObject")?,
            clip: fields.pptr("m_Animation")?,
            clips: fields.pptrs("m_Animations")?,
            wrap_mode: fields.i32("m_WrapMode")?,
            play_automatically: fields.bool("m_PlayAutomatically")?,
        })
    }
}
