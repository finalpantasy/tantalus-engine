use tantalus_assets::pc::serialized::SerializedFile;

fn be(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn le(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn fixture(type_tree: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    be(&mut bytes, 0);
    be(&mut bytes, 0);
    be(&mut bytes, 15);
    be(&mut bytes, 0);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(b"5.2.3p2\0");
    le(&mut bytes, 5);
    bytes.push(u8::from(type_tree));
    le(&mut bytes, 1);
    le(&mut bytes, 49);
    bytes.extend_from_slice(&[0; 16]);
    if type_tree {
        le(&mut bytes, 1);
        le(&mut bytes, 0);
        bytes.extend_from_slice(&[0; 24]);
    }
    le(&mut bytes, 1);
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    bytes.extend_from_slice(&7_i64.to_le_bytes());
    le(&mut bytes, 0);
    le(&mut bytes, 4);
    le(&mut bytes, 49);
    bytes.extend_from_slice(&49_u16.to_le_bytes());
    bytes.extend_from_slice(&0_i16.to_le_bytes());
    bytes.push(0);
    le(&mut bytes, 0);
    le(&mut bytes, 0);
    bytes.push(0);
    let data_offset = bytes.len() as u32;
    bytes[12..16].copy_from_slice(&data_offset.to_be_bytes());
    bytes.extend_from_slice(b"test");
    let file_size = bytes.len() as u32;
    bytes[4..8].copy_from_slice(&file_size.to_be_bytes());
    bytes[0..4].copy_from_slice(&(data_offset - 24).to_be_bytes());
    bytes
}

#[test]
fn format_15_object_table_reads_real_layout_rules() {
    for type_tree in [false, true] {
        let bytes = fixture(type_tree);
        let file = SerializedFile::parse(&bytes, "synthetic").unwrap();
        let report = file.report();
        assert_eq!(report.format_version, 15);
        assert_eq!(report.unity_version, "5.2.3p2");
        assert_eq!(report.target_platform, 5);
        assert_eq!(report.type_trees_embedded, type_tree);
        assert_eq!(report.types[0].class_id, 49);
        assert_eq!(report.types[0].tree_nodes, u32::from(type_tree));
        assert_eq!(report.objects.len(), 1);
        assert_eq!(report.objects[0].path_id, 7);
        assert_eq!(report.objects[0].class_id, 49);
        assert_eq!(file.object_data(0).unwrap(), b"test");
    }
}

#[test]
fn unsupported_version_truncation_and_unknown_types_fail() {
    let mut bytes = fixture(false);
    bytes[8..12].copy_from_slice(&14_u32.to_be_bytes());
    assert!(SerializedFile::parse(&bytes, "version").is_err());

    let mut bytes = fixture(false);
    bytes.truncate(bytes.len() - 2);
    assert!(SerializedFile::parse(&bytes, "truncated").is_err());

    let mut bytes = fixture(false);
    let position = bytes
        .windows(4)
        .position(|window| window == 49_i32.to_le_bytes())
        .unwrap();
    bytes[position..position + 4].copy_from_slice(&50_i32.to_le_bytes());
    assert!(SerializedFile::parse(&bytes, "unknown type").is_err());
}
