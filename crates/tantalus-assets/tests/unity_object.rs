use std::path::PathBuf;
use tantalus_assets::pc::serialized::{RawTypeTree, SerializedFile};
use tantalus_assets::pc::unity_object::{
    AnimationClip, Mesh, TypeTree, UnityObject, Value, decode_object, inspect_unity_objects,
};
use tantalus_assets::pc::unity_raw::UnityRawBundle;

const ALIGN: i32 = 0x4000;

/// Builds a type tree whose strings all live in the local buffer, except
/// entries given as `@<offset>` which reference the common string table.
struct TreeBuilder {
    nodes: Vec<u8>,
    strings: Vec<u8>,
    count: u32,
}

impl TreeBuilder {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            strings: Vec::new(),
            count: 0,
        }
    }

    fn string(&mut self, value: &str) -> i32 {
        if let Some(offset) = value.strip_prefix('@') {
            return (0x8000_0000_u32 | offset.parse::<u32>().unwrap()) as i32;
        }
        let offset = self.strings.len() as i32;
        self.strings.extend_from_slice(value.as_bytes());
        self.strings.push(0);
        offset
    }

    fn node(
        mut self,
        depth: u8,
        kind: &str,
        name: &str,
        size: i32,
        array: bool,
        meta: i32,
    ) -> Self {
        let kind = self.string(kind);
        let name = self.string(name);
        self.nodes.extend_from_slice(&1_u16.to_le_bytes());
        self.nodes.push(depth);
        self.nodes.push(u8::from(array));
        for word in [kind, name, size, self.count as i32, meta] {
            self.nodes.extend_from_slice(&word.to_le_bytes());
        }
        self.count += 1;
        self
    }

    fn build(&self) -> TypeTree {
        TypeTree::parse(
            &RawTypeTree {
                node_count: self.count,
                nodes: &self.nodes,
                strings: &self.strings,
            },
            "fixture",
        )
        .unwrap()
    }
}

fn sample_tree() -> TypeTree {
    TreeBuilder::new()
        .node(0, "Thing", "Base", -1, false, 0)
        .node(1, "bool", "m_Flag", 1, false, ALIGN)
        .node(1, "vector", "m_Values", -1, false, 0)
        .node(2, "Array", "Array", -1, true, 0)
        .node(3, "int", "size", 4, false, 0)
        .node(3, "float", "data", 4, false, 0)
        .node(1, "string", "m_Name", -1, false, 0)
        .node(2, "Array", "Array", -1, true, ALIGN)
        .node(3, "int", "size", 4, false, 0)
        .node(3, "char", "data", 1, false, 0)
        .node(1, "vector", "m_Pairs", -1, false, 0)
        .node(2, "Array", "Array", -1, true, 0)
        .node(3, "int", "size", 4, false, 0)
        .node(3, "pair", "data", 12, false, 0)
        .node(4, "int", "first", 4, false, 0)
        .node(4, "SInt64", "second", 8, false, 0)
        .build()
}

fn sample_bytes() -> Vec<u8> {
    let mut bytes = vec![1, 0, 0, 0];
    bytes.extend_from_slice(&2_i32.to_le_bytes());
    bytes.extend_from_slice(&1.5_f32.to_le_bytes());
    bytes.extend_from_slice(&(-2.0_f32).to_le_bytes());
    bytes.extend_from_slice(&3_i32.to_le_bytes());
    bytes.extend_from_slice(b"abc\0");
    bytes.extend_from_slice(&1_i32.to_le_bytes());
    bytes.extend_from_slice(&7_i32.to_le_bytes());
    bytes.extend_from_slice(&(-9_i64).to_le_bytes());
    bytes
}

#[test]
fn type_tree_reads_aligned_struct_packed_arrays_and_strings_exactly() {
    let tree = sample_tree();
    assert_eq!(tree.root_type(), "Thing");
    let value = tree.read(&sample_bytes(), "fixture").unwrap();
    assert_eq!(value.field("m_Flag"), Some(&Value::Bool(true)));
    assert_eq!(value.field("m_Values"), Some(&Value::F32s(vec![1.5, -2.0])));
    assert_eq!(value.field("m_Name").and_then(Value::as_str), Some("abc"));
    let pairs = value.field("m_Pairs").and_then(Value::as_array).unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].field("first").and_then(Value::as_i64), Some(7));
    assert_eq!(pairs[0].field("second").and_then(Value::as_i64), Some(-9));
}

#[test]
fn type_tree_rejects_unconsumed_and_truncated_object_bytes() {
    let tree = sample_tree();
    let mut long = sample_bytes();
    long.push(0);
    let error = tree.read(&long, "fixture").unwrap_err().to_string();
    assert!(error.contains("consumed 40 of 41"), "{error}");
    let mut short = sample_bytes();
    short.truncate(30);
    assert!(tree.read(&short, "fixture").is_err());
    let mut huge = sample_bytes();
    huge[4..8].copy_from_slice(&1_000_i32.to_le_bytes());
    let error = tree.read(&huge, "fixture").unwrap_err().to_string();
    assert!(error.contains("exceeds"), "{error}");
}

#[test]
fn type_tree_resolves_common_strings_and_keeps_binary_strings_as_bytes() {
    // Offsets 0 ("AABB") and 49 ("Array") in Unity's common string table.
    let tree = TreeBuilder::new()
        .node(0, "@0", "Base", -1, false, 0)
        .node(1, "string", "m_Script", -1, false, 0)
        .node(2, "@49", "Array", -1, true, ALIGN)
        .node(3, "int", "size", 4, false, 0)
        .node(3, "char", "data", 1, false, 0)
        .build();
    assert_eq!(tree.root_type(), "AABB");
    assert_eq!(tree.nodes[2].type_name, "Array");
    let mut bytes = 2_i32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&[0xff, 0xfe, 0, 0]);
    let value = tree.read(&bytes, "fixture").unwrap();
    assert_eq!(
        value.field("m_Script").and_then(Value::as_bytes),
        Some(&[0xff, 0xfe][..])
    );
}

fn uint(value: u64) -> Value<'static> {
    Value::UInt(value)
}

fn float(value: f32) -> Value<'static> {
    Value::Float(f64::from(value))
}

fn object(fields: Vec<(&'static str, Value<'static>)>) -> Value<'static> {
    Value::Struct(fields)
}

fn vec3(value: [f32; 3]) -> Value<'static> {
    object(vec![
        ("x", float(value[0])),
        ("y", float(value[1])),
        ("z", float(value[2])),
    ])
}

fn aabb() -> Value<'static> {
    object(vec![
        ("m_Center", vec3([0.0; 3])),
        ("m_Extent", vec3([1.0; 3])),
    ])
}

fn packed(items: u64) -> Value<'static> {
    object(vec![
        ("m_NumItems", uint(items)),
        ("m_Data", Value::Bytes(Vec::new())),
        ("m_BitSize", uint(0)),
    ])
}

/// Three-vertex, one-triangle skinned mesh with position (stream 0) and
/// uv0 (stream 1) channels.
fn mesh_value(indices: [u16; 3], bone: i64) -> Value<'static> {
    let channel = |stream: u64, offset: u64, dimension: u64| {
        object(vec![
            ("stream", uint(stream)),
            ("offset", uint(offset)),
            ("format", uint(0)),
            ("dimension", uint(dimension)),
        ])
    };
    let mut blob = Vec::new();
    for vertex in 0..3 {
        for component in 0..3 {
            blob.extend_from_slice(&((vertex * 3 + component) as f32 * 0.1).to_le_bytes());
        }
    }
    blob.extend_from_slice(&[0; 12]); // pad stream 0 (36 bytes) to 48
    blob.extend_from_slice(&[0; 24]); // stream 1: 3 * float2
    let mut index_buffer = Vec::new();
    for index in indices {
        index_buffer.extend_from_slice(&index.to_le_bytes());
    }
    let influence = object(vec![
        ("weight[0]", float(1.0)),
        ("weight[1]", float(0.0)),
        ("weight[2]", float(0.0)),
        ("weight[3]", float(0.0)),
        ("boneIndex[0]", Value::Int(bone)),
        ("boneIndex[1]", Value::Int(0)),
        ("boneIndex[2]", Value::Int(0)),
        ("boneIndex[3]", Value::Int(0)),
    ]);
    let mut matrix = Vec::new();
    for (index, name) in [
        "e00", "e01", "e02", "e03", "e10", "e11", "e12", "e13", "e20", "e21", "e22", "e23", "e30",
        "e31", "e32", "e33",
    ]
    .into_iter()
    .enumerate()
    {
        matrix.push((name, float(if index % 5 == 0 { 1.0 } else { 0.0 })));
    }
    object(vec![
        ("m_Name", Value::String("tri".to_owned())),
        (
            "m_SubMeshes",
            Value::Array(vec![object(vec![
                ("firstByte", uint(0)),
                ("indexCount", uint(3)),
                ("topology", Value::Int(0)),
                ("firstVertex", uint(0)),
                ("vertexCount", uint(3)),
                ("localAABB", aabb()),
            ])]),
        ),
        (
            "m_Shapes",
            object(vec![
                ("vertices", Value::Array(Vec::new())),
                ("shapes", Value::Array(Vec::new())),
                ("channels", Value::Array(Vec::new())),
                ("fullWeights", Value::F32s(Vec::new())),
            ]),
        ),
        ("m_BindPose", Value::Array(vec![object(matrix)])),
        ("m_BoneNameHashes", Value::U32s(vec![42])),
        ("m_RootBoneNameHash", uint(42)),
        ("m_MeshCompression", uint(0)),
        ("m_IndexBuffer", Value::Bytes(index_buffer)),
        ("m_Skin", Value::Array(vec![influence; 3])),
        (
            "m_VertexData",
            object(vec![
                ("m_CurrentChannels", uint(0b1001)),
                ("m_VertexCount", uint(3)),
                (
                    "m_Channels",
                    Value::Array(vec![
                        channel(0, 0, 3),
                        channel(0, 0, 0),
                        channel(0, 0, 0),
                        channel(1, 0, 2),
                    ]),
                ),
                ("m_DataSize", Value::Bytes(blob)),
            ]),
        ),
        ("m_CompressedMesh", object(vec![("m_Vertices", packed(0))])),
        ("m_LocalAABB", aabb()),
    ])
}

#[test]
fn mesh_derives_padded_streams_and_validates_indices_and_bones() {
    let mesh = Mesh::from_value(&mesh_value([0, 1, 2], 0), "fixture").unwrap();
    assert_eq!(mesh.vertex_count, 3);
    assert_eq!(mesh.streams.len(), 2);
    assert_eq!((mesh.streams[0].offset, mesh.streams[0].stride), (0, 12));
    assert_eq!((mesh.streams[1].offset, mesh.streams[1].stride), (48, 8));
    assert_eq!(mesh.streams[1].channel_mask, 0b1000);
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert_eq!(mesh.bind_poses.len(), 1);
    assert_eq!(mesh.bind_poses[0][0], 1.0);
    assert_eq!(mesh.skin.len(), 3);
    assert_eq!(mesh.layout_signature(), "position:f32x3@0/0,uv0:f32x2@1/0");
    let positions = mesh.channel_f32(0).unwrap();
    assert_eq!(positions.len(), 9);
    assert!((positions[4] - 0.4).abs() < 1e-6);

    let error = Mesh::from_value(&mesh_value([0, 1, 3], 0), "fixture")
        .unwrap_err()
        .to_string();
    assert!(error.contains("index 3 is out of range"), "{error}");
    let error = Mesh::from_value(&mesh_value([0, 1, 2], 1), "fixture")
        .unwrap_err()
        .to_string();
    assert!(error.contains("weights bone 1"), "{error}");
}

fn quat_key(time: f32) -> Value<'static> {
    let quat = || {
        object(vec![
            ("x", float(0.0)),
            ("y", float(0.0)),
            ("z", float(0.0)),
            ("w", float(1.0)),
        ])
    };
    object(vec![
        ("time", float(time)),
        ("value", quat()),
        ("inSlope", quat()),
        ("outSlope", quat()),
    ])
}

fn clip_value(times: &[f32]) -> Value<'static> {
    let empty = || Value::Array(Vec::new());
    let xform = || object(Vec::new());
    object(vec![
        ("m_Name", Value::String("idle".to_owned())),
        ("m_Legacy", Value::Bool(true)),
        ("m_Compressed", Value::Bool(false)),
        ("m_UseHighQualityCurve", Value::Bool(true)),
        (
            "m_RotationCurves",
            Value::Array(vec![object(vec![
                (
                    "curve",
                    object(vec![
                        (
                            "m_Curve",
                            Value::Array(times.iter().map(|time| quat_key(*time)).collect()),
                        ),
                        ("m_PreInfinity", Value::Int(2)),
                        ("m_PostInfinity", Value::Int(2)),
                    ]),
                ),
                ("path", Value::String("bone000".to_owned())),
            ])]),
        ),
        ("m_CompressedRotationCurves", empty()),
        ("m_PositionCurves", empty()),
        ("m_ScaleCurves", empty()),
        ("m_FloatCurves", empty()),
        ("m_PPtrCurves", empty()),
        ("m_SampleRate", float(30.0)),
        ("m_WrapMode", Value::Int(0)),
        ("m_Bounds", aabb()),
        ("m_MuscleClipSize", uint(0)),
        (
            "m_MuscleClip",
            object(vec![
                ("m_StartX", xform()),
                (
                    "m_Clip",
                    object(vec![(
                        "data",
                        object(vec![
                            (
                                "m_StreamedClip",
                                object(vec![("data", Value::U32s(Vec::new()))]),
                            ),
                            ("m_DenseClip", object(vec![("m_FrameCount", Value::Int(0))])),
                            (
                                "m_ConstantClip",
                                object(vec![("data", Value::F32s(Vec::new()))]),
                            ),
                        ]),
                    )]),
                ),
                ("m_StartTime", float(0.0)),
                ("m_StopTime", float(0.0)),
            ]),
        ),
        (
            "m_ClipBindingConstant",
            object(vec![("genericBindings", empty())]),
        ),
        ("m_Events", empty()),
    ])
}

#[test]
fn animation_clip_reads_curves_length_and_rejects_unordered_keys() {
    let clip = AnimationClip::from_value(&clip_value(&[0.0, 0.5, 1.0]), "fixture").unwrap();
    assert_eq!(clip.name, "idle");
    assert_eq!(clip.rotation_curves.len(), 1);
    assert_eq!(clip.rotation_curves[0].path, "bone000");
    assert_eq!(clip.rotation_curves[0].keys[1].value, [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(clip.key_count(), 3);
    assert_eq!(clip.length(), 1.0);
    assert_eq!(clip.frame_count(), 31);
    let error = AnimationClip::from_value(&clip_value(&[0.0, 1.0, 0.5]), "fixture")
        .unwrap_err()
        .to_string();
    assert!(error.contains("non-decreasing"), "{error}");
}

fn install_bundle(name: &str) -> Option<PathBuf> {
    let root = std::env::var_os("TANTALUS_FFIX_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX"));
    let path = root.join("StreamingAssets").join(name);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!("skipping: {} is absent", path.display());
        None
    }
}

#[test]
fn real_install_models_bundle_decodes_every_object_exactly() {
    let Some(path) = install_bundle("p0data4.bin") else {
        return;
    };
    let bundle = UnityRawBundle::open(&path).unwrap();
    let bytes = bundle.read_entry(0, 600 * 1024 * 1024).unwrap();
    let file = SerializedFile::parse(&bytes, "p0data4").unwrap();
    let report = inspect_unity_objects(&file, None, "p0data4").unwrap();
    assert!(report.type_trees_embedded);
    assert_eq!(report.objects_selected, 40_130);
    for class in &report.classes {
        assert_eq!(class.failed, 0, "{class:?}");
    }
    let counts: Vec<(u16, usize)> = report
        .classes
        .iter()
        .map(|class| (class.class_id, class.parsed))
        .collect();
    assert!(counts.contains(&(43, 1_586)));
    assert!(counts.contains(&(137, 1_586)));
    assert!(counts.contains(&(4, 16_645)));
    let mesh = report.mesh.unwrap();
    assert_eq!(mesh.meshes, 1_586);
    assert_eq!(mesh.compressed_meshes, 0);
    assert_eq!(mesh.positions_outside_local_aabb, 0);
    assert_eq!(mesh.influences_with_bad_weight_sum, 0);
    let hierarchy = report.hierarchy.unwrap();
    assert_eq!(hierarchy.broken_transform_links, 0);
    assert_eq!(hierarchy.broken_component_links, 0);
    assert_eq!(hierarchy.renderer_mesh_unresolved, 0);
    assert_eq!(hierarchy.renderer_bone_unresolved, 0);
    assert_eq!(hierarchy.renderer_bone_count_mismatch, 0);
}

#[test]
fn real_install_animation_bundle_decodes_clips_exactly() {
    let Some(path) = install_bundle("p0data5.bin") else {
        return;
    };
    let bundle = UnityRawBundle::open(&path).unwrap();
    let bytes = bundle.read_entry(0, 600 * 1024 * 1024).unwrap();
    let file = SerializedFile::parse(&bytes, "p0data5").unwrap();
    let objects = &file.report().objects;
    assert_eq!(
        objects
            .iter()
            .filter(|object| object.class_id == 74)
            .count(),
        9_278
    );
    let clip_type = objects.iter().find(|o| o.class_id == 74).unwrap().type_id;
    let tree = TypeTree::parse(&file.type_tree(clip_type).unwrap(), "p0data5").unwrap();
    let mut checked = 0;
    for (index, object) in objects.iter().enumerate() {
        if object.class_id != 74 {
            continue;
        }
        let decoded = decode_object(&file, &tree, index, "p0data5").unwrap();
        let UnityObject::AnimationClip(clip) = decoded else {
            panic!("class 74 must decode as an AnimationClip");
        };
        assert!(clip.legacy && clip.key_count() > 0, "{}", clip.name);
        checked += 1;
    }
    assert_eq!(checked, 9_278);
}
