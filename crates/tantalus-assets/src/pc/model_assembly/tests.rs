//! Hand-built fixtures: a three-node prefab with one skinned part.

use super::pose::{
    IDENTITY, Interpolation, NodePose, bind_clip, mat_affine_inverse, mat_from_pose, mat_mul,
    normalize_quat, rest_pose, sample_clip, sample_curve, skin_model, skinned_bounds,
};
use super::*;
use crate::pc::texture::TextureSettings;
use crate::pc::unity_object::Transform;
use crate::pc::unity_object::animation::{Curve, Keyframe};
use crate::pc::unity_object::mesh::{SubMesh, VertexChannel, VertexStream};
use crate::pc::unity_object::scene::AnimationComponent;
use std::cell::RefCell;

#[derive(Default)]
struct FixtureStore {
    objects: HashMap<i64, UnityObject>,
    textures: HashMap<i64, Texture2D>,
    loads: RefCell<usize>,
}

impl ObjectStore for FixtureStore {
    fn load(&self, path_id: i64) -> Result<Option<UnityObject>, ImportError> {
        *self.loads.borrow_mut() += 1;
        Ok(self.objects.get(&path_id).cloned())
    }

    fn texture(&self, path_id: i64) -> Result<Option<Texture2D>, ImportError> {
        Ok(self.textures.get(&path_id).cloned())
    }
}

fn ptr(path_id: i64) -> PPtr {
    PPtr {
        file_id: 0,
        path_id,
    }
}

fn game_object(name: &str, components: &[(i32, i64)]) -> UnityObject {
    UnityObject::GameObject(GameObject {
        name: name.to_owned(),
        layer: 0,
        tag: 0,
        active: true,
        components: components.iter().map(|(c, id)| (*c, ptr(*id))).collect(),
    })
}

fn transform(owner: i64, father: i64, children: &[i64], position: [f32; 3]) -> UnityObject {
    UnityObject::Transform(Transform {
        game_object: ptr(owner),
        local_rotation: [0.0, 0.0, 0.0, 1.0],
        local_position: position,
        local_scale: [1.0, 1.0, 1.0],
        children: children.iter().map(|id| ptr(*id)).collect(),
        father: ptr(father),
    })
}

fn position_only_mesh() -> Mesh {
    let positions: [[f32; 3]; 3] = [[0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [0.0, 2.0, 0.0]];
    let mut blob = Vec::new();
    for position in positions {
        for value in position {
            blob.extend_from_slice(&value.to_le_bytes());
        }
    }
    let mut channels = vec![
        VertexChannel {
            stream: 0,
            offset: 0,
            format: 0,
            dimension: 0
        };
        8
    ];
    channels[0].dimension = 3;
    // Vertex 0 fully on bone 0 (hip), vertex 1 half on each bone, vertex 2 on bone 1 (root).
    let skin = vec![
        BoneInfluence {
            weights: [1.0, 0.0, 0.0, 0.0],
            bones: [0, 0, 0, 0],
        },
        BoneInfluence {
            weights: [0.5, 0.5, 0.0, 0.0],
            bones: [0, 1, 0, 0],
        },
        BoneInfluence {
            weights: [1.0, 0.0, 0.0, 0.0],
            bones: [1, 0, 0, 0],
        },
    ];
    let aabb = Aabb {
        center: [0.5, 1.5, 0.0],
        extent: [0.5, 0.5, 0.0],
    };
    // Bone 0 = hip at (0,1,0): bind = inverse translation; bone 1 = root: identity.
    let mut hip_bind = IDENTITY;
    hip_bind[7] = -1.0;
    Mesh {
        name: "body_mesh".to_owned(),
        vertex_count: 3,
        current_channels: 1,
        channels,
        streams: vec![VertexStream {
            channel_mask: 1,
            offset: 0,
            stride: 12,
        }],
        vertex_data: blob,
        indices: vec![0, 1, 2],
        submeshes: vec![SubMesh {
            first_byte: 0,
            index_count: 3,
            topology: 0,
            first_vertex: 0,
            vertex_count: 3,
            local_aabb: aabb,
        }],
        bind_poses: vec![hip_bind, IDENTITY],
        bone_name_hashes: vec![],
        root_bone_name_hash: 0,
        skin,
        blend_shape_vertices: 0,
        blend_shape_channels: 0,
        mesh_compression: 0,
        compressed_vertex_items: 0,
        local_aabb: aabb,
    }
}

fn dxt5_red() -> Texture2D {
    let mut block = vec![255, 255, 0, 0, 0, 0, 0, 0];
    block.extend_from_slice(&0xF800_u16.to_le_bytes());
    block.extend_from_slice(&0xF800_u16.to_le_bytes());
    block.extend_from_slice(&[0; 4]);
    texture("red", 12, block)
}

fn texture(name: &str, format: i32, image_data: Vec<u8>) -> Texture2D {
    Texture2D {
        name: name.to_owned(),
        width: 4,
        height: 4,
        complete_image_size: image_data.len() as i32,
        format,
        mip_count: 1,
        is_readable: false,
        read_allowed: false,
        image_count: 1,
        texture_dimension: 2,
        settings: TextureSettings {
            filter_mode: 1,
            anisotropy: 1,
            mip_bias: 0.0,
            wrap_mode: 0,
        },
        lightmap_format: 0,
        color_space: 0,
        image_data,
    }
}

fn material(textures: &[(&str, i64)]) -> UnityObject {
    UnityObject::Material(Material {
        name: "mat".to_owned(),
        shader: ptr(0),
        shader_keywords: String::new(),
        textures: textures
            .iter()
            .map(|(name, id)| ((*name).to_owned(), ptr(*id)))
            .collect(),
        floats: vec![],
        colors: vec![],
    })
}

/// root(1) -> hip(2) at (0,1,0), body(3) carrying the renderer.
fn fixture(bones: Vec<PPtr>, material_slots: &[(&str, i64)]) -> FixtureStore {
    let mut store = FixtureStore::default();
    let objects = &mut store.objects;
    objects.insert(1, game_object("root", &[(4, 101), (111, 150)]));
    objects.insert(2, game_object("hip", &[(4, 102)]));
    objects.insert(3, game_object("body", &[(4, 103), (137, 110)]));
    objects.insert(101, transform(1, 0, &[102, 103], [0.0; 3]));
    objects.insert(102, transform(2, 101, &[], [0.0, 1.0, 0.0]));
    objects.insert(103, transform(3, 101, &[], [0.0; 3]));
    objects.insert(
        110,
        UnityObject::SkinnedMeshRenderer(SkinnedMeshRenderer {
            game_object: ptr(3),
            enabled: true,
            materials: vec![ptr(130)],
            mesh: ptr(120),
            bones,
            root_bone: ptr(102),
            aabb: Aabb {
                center: [0.5, 1.5, 0.0],
                extent: [0.5, 0.5, 0.0],
            },
            blend_shape_weights: vec![],
            update_when_offscreen: false,
        }),
    );
    objects.insert(120, UnityObject::Mesh(Box::new(position_only_mesh())));
    objects.insert(130, material(material_slots));
    objects.insert(
        150,
        UnityObject::Animation(AnimationComponent {
            game_object: ptr(1),
            clip: ptr(160),
            clips: vec![ptr(160)],
            wrap_mode: 0,
            play_automatically: false,
        }),
    );
    store.textures.insert(140, dxt5_red());
    store.textures.insert(141, texture("raw", 4, vec![0; 64]));
    store
}

fn good_store() -> FixtureStore {
    fixture(
        vec![ptr(102), ptr(101)],
        &[("_MainTex", 140), ("_Other", 141)],
    )
}

#[test]
fn assembles_nodes_bones_materials_and_textures() {
    let store = good_store();
    let model = assemble_model(&store, "fixture", 1, AssembleOptions::default()).unwrap();
    let paths: Vec<&str> = model.nodes.iter().map(|n| n.path.as_str()).collect();
    assert_eq!(paths, ["", "hip", "body"]);
    assert_eq!(model.nodes[1].parent, Some(0));
    assert_eq!(model.nodes[0].children, [1, 2]);
    assert_eq!(model.embedded_clips, [160]);
    let part = &model.parts[0];
    assert_eq!(part.node, 2);
    assert_eq!(part.bones, [Some(1), Some(0)]);
    assert_eq!(part.indices, [0, 1, 2]);
    assert_eq!(part.positions[2], [0.0, 2.0, 0.0]);
    assert_eq!(model.issues.total(), 0, "{:?}", model.issues);
    assert!(model.rest_skin_error < 1e-5, "{}", model.rest_skin_error);
    let bounds = model.bind_bounds.unwrap();
    assert_eq!(bounds.center, [0.5, 1.5, 0.0]);

    let material = part.submeshes[0].material.as_ref().unwrap();
    assert_eq!(material.textures.len(), 2);
    let decoded = &material.textures[0];
    assert_eq!(decoded.rgba.as_ref().unwrap().len(), 64);
    assert_eq!(decoded.rgba.as_ref().unwrap()[3], 255);
    let raw = &material.textures[1];
    assert!(raw.rgba.is_none());
    assert!(raw.undecodable.as_ref().unwrap().contains("format 4"));

    let summary = model.summary();
    assert_eq!(summary.distinct_bones, 2);
    assert_eq!(summary.textures_decoded, 1);
    assert_eq!(summary.textures_undecodable, 1);
    let mut stripped = model.clone();
    stripped.drop_pixels();
    assert_eq!(stripped.summary().textures_decoded, 1);
}

#[test]
fn skips_pixel_decode_when_asked_but_keeps_the_verdict() {
    let store = good_store();
    let options = AssembleOptions {
        decode_textures: false,
    };
    let model = assemble_model(&store, "fixture", 1, options).unwrap();
    let summary = model.summary();
    assert_eq!(summary.textures_decoded, 1);
    assert_eq!(summary.textures_undecodable, 1);
    let first = &model.parts[0].submeshes[0]
        .material
        .as_ref()
        .unwrap()
        .textures[0];
    assert!(first.rgba.is_none() && first.undecodable.is_none());
}

#[test]
fn reports_unresolved_bones_and_missing_textures() {
    let store = fixture(
        vec![ptr(102), ptr(999)],
        &[("_MainTex", 140), ("_Gone", 555)],
    );
    let model = assemble_model(&store, "fixture", 1, AssembleOptions::default()).unwrap();
    assert_eq!(model.parts[0].bones, [Some(1), None]);
    assert_eq!(model.issues.bone_unresolved, 1);
    assert_eq!(model.issues.texture_unresolved, 1);
    assert!(model.issues.samples.iter().any(|s| s.contains("bone")));
    // The surviving influences still skin; vertex 2 only used the lost bone.
    let posed = skin_model(&model, &rest_pose(&model));
    assert_eq!(posed[0][2], [0.0, 2.0, 0.0]);
}

#[test]
fn rejects_a_root_that_is_not_a_game_object() {
    let store = good_store();
    let error = assemble_model(&store, "fixture", 101, AssembleOptions::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("not a GameObject"), "{error}");
}

fn key<const N: usize>(time: f32, value: [f32; N], slope: f32) -> Keyframe<N> {
    Keyframe {
        time,
        value,
        in_slope: [slope; N],
        out_slope: [slope; N],
    }
}

fn curve<const N: usize>(path: &str, keys: Vec<Keyframe<N>>) -> Curve<N> {
    Curve {
        path: path.to_owned(),
        attribute: String::new(),
        class_id: 0,
        pre_infinity: 2,
        post_infinity: 2,
        keys,
    }
}

fn clip(
    rotation: Vec<Curve<4>>,
    position: Vec<Curve<3>>,
    scale: Vec<Curve<3>>,
    floats: Vec<Curve<1>>,
) -> AnimationClip {
    AnimationClip {
        name: "fixture".to_owned(),
        legacy: true,
        compressed: false,
        use_high_quality_curve: true,
        sample_rate: 30.0,
        wrap_mode: 0,
        bounds: Aabb {
            center: [0.0; 3],
            extent: [0.0; 3],
        },
        rotation_curves: rotation,
        position_curves: position,
        scale_curves: scale,
        float_curves: floats,
        compressed_rotation_curves: 0,
        pptr_curves: 0,
        muscle_clip_size: 0,
        muscle_start_time: 0.0,
        muscle_stop_time: 0.0,
        streamed_clip_words: 0,
        dense_frame_count: 0,
        constant_clip_values: 0,
        generic_bindings: 0,
        events: 0,
    }
}

#[test]
fn curve_sampling_interpolates_clamps_and_steps() {
    let keys = [key(0.0, [0.0], 2.0), key(1.0, [2.0], 2.0)];
    assert_eq!(
        sample_curve(&keys, -1.0, Interpolation::Linear),
        Some([0.0])
    );
    assert_eq!(sample_curve(&keys, 5.0, Interpolation::Linear), Some([2.0]));
    assert_eq!(
        sample_curve(&keys, 0.25, Interpolation::Linear),
        Some([0.5])
    );
    // Slopes of 2 over a delta of 2 are linear, so Hermite agrees with lerp.
    let hermite = sample_curve(&keys, 0.25, Interpolation::Hermite).unwrap()[0];
    assert!((hermite - 0.5).abs() < 1e-6, "{hermite}");
    let stepped = [
        key(0.0, [1.0], f32::INFINITY),
        key(1.0, [3.0], f32::INFINITY),
    ];
    assert_eq!(
        sample_curve(&stepped, 0.5, Interpolation::Hermite),
        Some([1.0])
    );
    let empty: [Keyframe<1>; 0] = [];
    assert_eq!(sample_curve(&empty, 0.0, Interpolation::Linear), None);
}

#[test]
fn curves_loop_and_ping_pong_outside_their_keys() {
    let mut looping = curve("a", vec![key(0.0, [0.0], 0.0), key(1.0, [1.0], 1.0)]);
    looping.keys[0].out_slope = [1.0];
    let at = |curve: &Curve<1>, time: f32| {
        pose::sample_full_curve(curve, time, Interpolation::Linear).unwrap()[0]
    };
    assert!((at(&looping, 1.25) - 0.25).abs() < 1e-5);
    assert!((at(&looping, -0.25) - 0.75).abs() < 1e-5);
    looping.pre_infinity = 4;
    looping.post_infinity = 4;
    assert!((at(&looping, 1.25) - 0.75).abs() < 1e-5);
    looping.post_infinity = 8;
    assert_eq!(at(&looping, 5.0), 1.0);
}

#[test]
fn matrices_compose_and_invert() {
    let pose = NodePose {
        position: [1.0, 2.0, 3.0],
        rotation: normalize_quat([
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        ]),
        scale: [2.0, 2.0, 2.0],
    };
    let matrix = mat_from_pose(&pose);
    let inverse = mat_affine_inverse(&matrix).unwrap();
    let product = mat_mul(&matrix, &inverse);
    for (got, want) in product.iter().zip(IDENTITY) {
        assert!((got - want).abs() < 1e-5, "{product:?}");
    }
    // 90 degrees about +Y turns +X into -Z.
    let unit = NodePose {
        position: [0.0; 3],
        rotation: [
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        ],
        scale: [1.0; 3],
    };
    let turned = pose::transform_point(&mat_from_pose(&unit), [1.0, 0.0, 0.0]);
    assert!(
        (turned[2] + 1.0).abs() < 1e-3 && turned[0].abs() < 1e-3,
        "{turned:?}"
    );
    assert!(mat_affine_inverse(&[0.0; 16]).is_none());
    assert_eq!(normalize_quat([0.0; 4]), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn binds_by_path_and_counts_unmatched_curves() {
    let store = good_store();
    let model = assemble_model(&store, "fixture", 1, AssembleOptions::default()).unwrap();
    let animation = clip(
        vec![curve("body", vec![key(0.0, [0.0, 0.0, 0.0, 1.0], 0.0)])],
        vec![
            curve("hip", vec![key(0.0, [0.0, 1.0, 0.0], 0.0)]),
            curve("", vec![key(0.0, [0.0; 3], 0.0)]),
            curve("hip/finger", vec![key(0.0, [0.0; 3], 0.0)]),
        ],
        vec![],
        vec![curve("hip", vec![key(0.0, [1.0], 0.0)])],
    );
    let binding = bind_clip(&model, &animation);
    assert_eq!((binding.matched, binding.unmatched), (3, 1));
    assert_eq!(binding.float_curves, 1);
    assert_eq!(binding.unmatched_paths, ["hip/finger"]);
    assert!(!binding.fully_matched());
}

#[test]
fn skinning_follows_the_sampled_pose() {
    let store = good_store();
    let model = assemble_model(&store, "fixture", 1, AssembleOptions::default()).unwrap();
    let animation = clip(
        vec![],
        vec![curve(
            "hip",
            vec![
                key(0.0, [0.0, 1.0, 0.0], 0.0),
                key(1.0, [2.0, 1.0, 0.0], 0.0),
            ],
        )],
        vec![],
        vec![],
    );
    let binding = bind_clip(&model, &animation);
    assert!(binding.fully_matched());

    // t = 0 equals the rest pose.
    let at_zero = sample_clip(&model, &animation, &binding, 0.0, Interpolation::Linear);
    assert_eq!(at_zero, rest_pose(&model));
    assert_eq!(skinned_bounds(&model, &at_zero), model.prefab_pose_bounds);
    assert_eq!(model.prefab_pose_bounds, model.bind_bounds);

    // Halfway the hip is at x = 1: vertex 0 (all hip) moves +1, vertex 1
    // (half hip) moves +0.5, vertex 2 (root) stays.
    let halfway = sample_clip(&model, &animation, &binding, 0.5, Interpolation::Linear);
    assert_eq!(halfway[1].position, [1.0, 1.0, 0.0]);
    let posed = skin_model(&model, &halfway);
    assert_eq!(posed[0][0], [1.0, 1.0, 0.0]);
    assert_eq!(posed[0][1], [1.5, 1.0, 0.0]);
    assert_eq!(posed[0][2], [0.0, 2.0, 0.0]);
    let bounds = skinned_bounds(&model, &halfway).unwrap();
    assert_eq!(bounds.center, [0.75, 1.5, 0.0]);
    let deviation = bounds_deviation(&model.bind_bounds.unwrap(), &bounds);
    assert!(deviation > 0.0 && deviation < 1.0, "{deviation}");
}

#[test]
fn rotation_samples_are_normalized() {
    let store = good_store();
    let model = assemble_model(&store, "fixture", 1, AssembleOptions::default()).unwrap();
    let animation = clip(
        vec![curve(
            "hip",
            vec![
                key(0.0, [0.0, 0.0, 0.0, 1.0], 0.0),
                key(1.0, [0.0, 1.0, 0.0, 1.0], 0.0),
            ],
        )],
        vec![],
        vec![],
        vec![],
    );
    let binding = bind_clip(&model, &animation);
    let poses = sample_clip(&model, &animation, &binding, 0.5, Interpolation::Linear);
    let q = poses[1].rotation;
    let length = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    assert!((length - 1.0).abs() < 1e-6, "{q:?}");
}

fn entry(path: &str, path_id: i64, class_id: u16) -> ContainerEntry {
    ContainerEntry {
        path: path.to_owned(),
        path_id,
        class_id,
    }
}

#[test]
fn catalog_groups_model_directories_and_finds_them() {
    let entries = [
        entry("assets/resources/models/2/98/98.fbx", 1, 1),
        entry("assets/resources/models/2/98/98.fbx", 2, 43),
        entry("assets/resources/models/2/98/98_0.png", 3, 28),
        entry("assets/resources/models/2/98/materials/98_0.mat", 4, 21),
        entry("assets/resources/models/3/98/98.fbx", 5, 1),
        entry("assets/resources/models/5/12/12.fbx", 6, 1),
        entry("assets/resources/models/geotexanim/x.tab.bytes", 7, 49),
        entry("assets/resources/models/4/7/walk.anim", 8, 74),
    ];
    let catalog = ModelCatalog::from_container(&entries);
    assert_eq!(catalog.dirs.len(), 4);
    assert_eq!(catalog.other_entries.len(), 1);
    let dir = &catalog.dirs["2/98"];
    assert_eq!(
        (dir.roots.clone(), dir.meshes, dir.textures, dir.materials),
        (vec![1], 1, 1, 1)
    );
    assert_eq!(dir.type_name, "main");
    assert_eq!(dir.geo_name, Some("GEO_MAIN_F0_ZDN"));
    assert_eq!(catalog.dirs["4/7"].clips, [8]);
    assert_eq!(catalog.find("2/98").unwrap().geo_id, 98);
    assert!(catalog.find("geo_main_f0_zdn").is_err());
    assert!(catalog.find("98").unwrap_err().contains("2 directories"));
    assert_eq!(catalog.find("12").unwrap().label, "5/12");
    assert!(catalog.find("nope").is_err());
    assert_eq!(catalog.dirs_for_geo(98).len(), 2);
}

#[test]
fn parses_animation_container_paths() {
    let entries = [
        entry("assets/resources/animations/98/1234.anim", 1, 74),
        entry("assets/resources/animations/0/5.anim", 2, 74),
        entry("assets/resources/animations/x/5.anim", 3, 74),
        entry("assets/resources/animations/1/a/b.anim", 4, 74),
        entry("assets/resources/animations/1/c.txt", 5, 49),
    ];
    let (clips, other) = clip_entries(&entries);
    assert_eq!(clips.len(), 2);
    assert_eq!((clips[0].geo_id, clips[0].name.as_str()), (98, "1234"));
    assert_eq!(other.len(), 3);
}

#[test]
fn geo_table_is_sorted_and_resolves_names() {
    assert!(geo_names::GEO_NAMES.windows(2).all(|w| w[0].0 < w[1].0));
    assert_eq!(geo_name(98), Some("GEO_MAIN_F0_ZDN"));
    assert_eq!(geo_name(999_999), None);
}
