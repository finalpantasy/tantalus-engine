//! Battle background (`BBG_Bnnn`) geometry: the static meshes of one
//! `battlemap_all/<bbg>/<bbg>` prefab of `p0data2.bin`, each with its world
//! matrix, decoded textures and the shader group Memoria gives it.
//!
//! Sources (Memoria, `Assembly-CSharp`):
//! - `HonoluluBattleMain.InitBattleScene` loads
//!   `BattleMap/BattleModel/battleMap_all/<bbg>/<bbg>` with
//!   `ModelFactory.CreateModel(path, Vector3.zero, Vector3.zero, true)`: the
//!   prefab sits at the origin, unrotated.
//! - `battlebg.SetDefaultShader` picks the shader from the name of each
//!   direct child of the root (`getBbgAttr`): `Group_0` plus, `Group_2`
//!   ground, `Group_4` minus, `Group_8` sky, anything else plus.
//! - `battlebg.SetMaterialShader`: a material whose name contains `a` takes
//!   the `_Abr_1` (additive) variant, one containing `s` the multiply one.
//!
//! The prefab renders through `MeshRenderer` + `MeshFilter`, which the
//! skinned-model assembler (`model_assembly`) skips, so this walks the
//! hierarchy itself.

use crate::install::ImportError;
use crate::pc::model_assembly::pose::{IDENTITY, Mat4, NodePose, mat_from_pose, mat_mul};
use crate::pc::model_assembly::{ObjectStore, SerializedStore, vertex_channel};
use crate::pc::unity_object::fields::Fields;
use crate::pc::unity_object::{PPtr, UnityObject};
use std::collections::HashMap;

/// `Transform`.
const CLASS_TRANSFORM: i32 = 4;
/// `MeshRenderer`.
const CLASS_MESH_RENDERER: i32 = 23;
/// `MeshFilter`.
const CLASS_MESH_FILTER: i32 = 33;

/// `battlebg.getBbgAttr`: the shader group of a root child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BbgGroup {
    /// `Group_8`: `PSX/BattleMap_Sky`, queue Background, no depth write.
    Sky,
    /// `Group_4`: `PSX/BattleMap_Minus`, queue AlphaTest.
    Minus,
    /// `Group_2`: `PSX/BattleMap_Ground`, queue AlphaTest+1, `_ZWrite` 0.
    Ground,
    /// `Group_0` and the rest: `PSX/BattleMap_Plus`, queue AlphaTest+3.
    Plus,
}

impl BbgGroup {
    pub fn of(name: &str) -> Self {
        match name {
            "Group_2" => Self::Ground,
            "Group_4" => Self::Minus,
            "Group_8" => Self::Sky,
            _ => Self::Plus,
        }
    }
}

/// How a material blends (`battlebg.SetMaterialShader`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BbgBlend {
    /// Cut out at alpha 0.5, opaque.
    Opaque,
    /// Name contains `a`: `_Abr_1`, `Blend One OneMinusSrcColor`, no depth
    /// write.
    Additive,
    /// Name contains `s`: `BattleMap_Plus_Abr_0_Multiply`.
    Multiply,
}

/// RGBA texels, rows top-down (`Texture2D::rgba_bytes`).
#[derive(Debug, Clone, PartialEq)]
pub struct BbgTexture {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BbgSubmesh {
    pub first_index: u32,
    pub index_count: u32,
    pub topology: i32,
    pub material: String,
    pub blend: BbgBlend,
    /// Index into [`BattleBackground::textures`].
    pub texture: Option<usize>,
}

/// One `MeshRenderer`.
#[derive(Debug, Clone, PartialEq)]
pub struct BbgMesh {
    pub name: String,
    pub group: BbgGroup,
    /// Node to prefab-root matrix (the root sits at the battle origin).
    pub world: Mat4,
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub submeshes: Vec<BbgSubmesh>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BattleBackground {
    pub label: String,
    pub meshes: Vec<BbgMesh>,
    pub textures: Vec<BbgTexture>,
    /// Objects that did not resolve (missing mesh, undecodable texture).
    pub problems: Vec<String>,
}

impl BattleBackground {
    pub fn triangle_count(&self) -> usize {
        self.meshes
            .iter()
            .flat_map(|mesh| &mesh.submeshes)
            .map(|submesh| submesh.index_count as usize / 3)
            .sum()
    }
}

/// Assembles the prefab whose root `GameObject` is `root`.
pub fn assemble_background(
    store: &SerializedStore<'_, '_>,
    label: &str,
    root: i64,
) -> Result<BattleBackground, ImportError> {
    let mut out = BattleBackground {
        label: label.to_owned(),
        ..BattleBackground::default()
    };
    let mut textures: HashMap<i64, Option<usize>> = HashMap::new();
    // (game object, parent world, group of the root child it is under, depth)
    let mut stack: Vec<(i64, Mat4, Option<BbgGroup>, usize)> = vec![(root, IDENTITY, None, 0)];
    while let Some((object, parent_world, group, depth)) = stack.pop() {
        if depth > 64 {
            return Err(ImportError::invalid(label, "hierarchy deeper than 64"));
        }
        let Some(UnityObject::GameObject(game_object)) = store.load(object)? else {
            out.problems.push(format!("{object} is not a GameObject"));
            continue;
        };
        let component = |class: i32| {
            game_object
                .components
                .iter()
                .find(|(id, _)| *id == class)
                .map(|(_, pointer)| *pointer)
        };
        let Some(transform_ref) = component(CLASS_TRANSFORM) else {
            continue;
        };
        let Some(UnityObject::Transform(transform)) = store.load(transform_ref.path_id)? else {
            out.problems
                .push(format!("`{}` has no Transform", game_object.name));
            continue;
        };
        let local = mat_from_pose(&NodePose {
            position: transform.local_position,
            rotation: transform.local_rotation,
            scale: transform.local_scale,
        });
        let world = if depth == 0 {
            // `CreateModel(..., Vector3.zero, Vector3.zero)` resets the root.
            IDENTITY
        } else {
            mat_mul(&parent_world, &local)
        };
        let group = match (depth, group) {
            (1, _) => Some(BbgGroup::of(&game_object.name)),
            (_, group) => group,
        };
        if let (Some(filter), Some(renderer)) =
            (component(CLASS_MESH_FILTER), component(CLASS_MESH_RENDERER))
        {
            let mesh_ref = store.read_value(filter.path_id, |value, source| {
                Fields::new(value, source, "MeshFilter").pptr("m_Mesh")
            })?;
            let renderer = store.read_value(renderer.path_id, |value, source| {
                let fields = Fields::new(value, source, "MeshRenderer");
                Ok((fields.bool("m_Enabled")?, fields.pptrs("m_Materials")?))
            })?;
            match (mesh_ref, renderer) {
                (Some(mesh_ref), Some((true, materials))) => {
                    let built = build_mesh(
                        store,
                        &game_object.name,
                        group.unwrap_or(BbgGroup::Plus),
                        world,
                        mesh_ref,
                        &materials,
                        &mut textures,
                        &mut out,
                    )?;
                    if let Some(mesh) = built {
                        out.meshes.push(mesh);
                    }
                }
                (_, Some((false, _))) => {}
                _ => out
                    .problems
                    .push(format!("`{}`: unreadable renderer", game_object.name)),
            }
        }
        for child in transform.children.iter().rev() {
            let Some(UnityObject::Transform(child)) = store.load(child.path_id)? else {
                continue;
            };
            stack.push((child.game_object.path_id, world, group, depth + 1));
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn build_mesh(
    store: &SerializedStore<'_, '_>,
    name: &str,
    group: BbgGroup,
    world: Mat4,
    mesh_ref: PPtr,
    materials: &[PPtr],
    textures: &mut HashMap<i64, Option<usize>>,
    out: &mut BattleBackground,
) -> Result<Option<BbgMesh>, ImportError> {
    let Some(UnityObject::Mesh(mesh)) = store.load(mesh_ref.path_id)? else {
        out.problems.push(format!("`{name}`: mesh {mesh_ref:?}"));
        return Ok(None);
    };
    let positions = vertex_channel(&mesh, 0, 3)
        .into_iter()
        .map(|v| [v[0], v[1], v[2]])
        .collect();
    let uvs = vertex_channel(&mesh, 3, 2)
        .into_iter()
        .map(|v| [v[0], v[1]])
        .collect();
    let mut submeshes = Vec::new();
    for (number, submesh) in mesh.submeshes.iter().enumerate() {
        let (material, texture) = match materials.get(number) {
            Some(pointer) if pointer.is_local() => match store.load(pointer.path_id)? {
                Some(UnityObject::Material(material)) => {
                    let main = material
                        .textures
                        .iter()
                        .find(|(property, _)| property == "_MainTex")
                        .or_else(|| material.textures.first())
                        .map(|(_, pointer)| *pointer)
                        .filter(|pointer| pointer.is_local() && !pointer.is_null());
                    let texture = match main {
                        Some(pointer) => match textures.get(&pointer.path_id) {
                            Some(slot) => *slot,
                            None => {
                                let slot = decode_texture(store, pointer.path_id, out);
                                textures.insert(pointer.path_id, slot);
                                slot
                            }
                        },
                        None => None,
                    };
                    (material.name, texture)
                }
                _ => (String::new(), None),
            },
            _ => (String::new(), None),
        };
        let blend = if material.contains('a') {
            BbgBlend::Additive
        } else if material.contains('s') {
            BbgBlend::Multiply
        } else {
            BbgBlend::Opaque
        };
        submeshes.push(BbgSubmesh {
            first_index: submesh.first_byte / 2,
            index_count: submesh.index_count,
            topology: submesh.topology,
            material,
            blend,
            texture,
        });
    }
    Ok(Some(BbgMesh {
        name: name.to_owned(),
        group,
        world,
        positions,
        uvs,
        indices: mesh.indices.iter().map(|&i| u32::from(i)).collect(),
        submeshes,
    }))
}

fn decode_texture(
    store: &SerializedStore<'_, '_>,
    path_id: i64,
    out: &mut BattleBackground,
) -> Option<usize> {
    let texture = match store.texture(path_id) {
        Ok(Some(texture)) => texture,
        Ok(None) => {
            out.problems.push(format!("texture {path_id} is missing"));
            return None;
        }
        Err(error) => {
            out.problems.push(format!("texture {path_id}: {error}"));
            return None;
        }
    };
    let (width, height) = (
        usize::try_from(texture.width).ok()?,
        usize::try_from(texture.height).ok()?,
    );
    match texture.rgba_bytes() {
        Ok(rgba) if width > 0 && height > 0 && rgba.len() >= width * height * 4 => {
            out.textures.push(BbgTexture {
                width,
                height,
                rgba,
            });
            Some(out.textures.len() - 1)
        }
        Ok(_) => {
            out.problems
                .push(format!("texture {path_id}: short pixels"));
            None
        }
        Err(error) => {
            out.problems.push(format!("texture {path_id}: {error}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_child_names_pick_the_shader_group() {
        assert_eq!(BbgGroup::of("Group_0"), BbgGroup::Plus);
        assert_eq!(BbgGroup::of("Group_2"), BbgGroup::Ground);
        assert_eq!(BbgGroup::of("Group_4"), BbgGroup::Minus);
        assert_eq!(BbgGroup::of("Group_8"), BbgGroup::Sky);
        assert_eq!(BbgGroup::of("Other"), BbgGroup::Plus);
    }
}
