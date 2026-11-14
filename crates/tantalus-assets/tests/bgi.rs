use tantalus_assets::pc::bgi::BgiFile;

fn u16le(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn i16le(bytes: &mut Vec<u8>, value: i16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn u32le(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn vec3(bytes: &mut Vec<u8>, values: [i16; 3]) {
    for value in values {
        i16le(bytes, value);
    }
}

fn fixture() -> Vec<u8> {
    let mut bytes = Vec::new();
    u32le(&mut bytes, 0xacdc_dead);
    u16le(&mut bytes, 202);
    for _ in 0..5 {
        vec3(&mut bytes, [0; 3]);
    }
    i16le(&mut bytes, 0);
    i16le(&mut bytes, 0);
    for (count, offset) in [(1, 60), (1, 100), (1, 170), (1, 104), (1, 136), (3, 152)] {
        u16le(&mut bytes, count);
        u16le(&mut bytes, offset);
    }

    u16le(&mut bytes, 1);
    u16le(&mut bytes, 2);
    for value in [0, 0, 0, 0] {
        i16le(&mut bytes, value);
    }
    for value in [0, 1, 2] {
        i16le(&mut bytes, value);
    }
    for value in [0, -1, -1] {
        i16le(&mut bytes, value);
    }
    for value in [-1, -1, -1] {
        i16le(&mut bytes, value);
    }
    vec3(&mut bytes, [1, 2, 3]);
    u32le(&mut bytes, 0);

    u16le(&mut bytes, 0);
    i16le(&mut bytes, -1);

    u16le(&mut bytes, 0);
    u16le(&mut bytes, 0);
    for _ in 0..4 {
        vec3(&mut bytes, [0; 3]);
    }
    u16le(&mut bytes, 1);
    u16le(&mut bytes, 198);

    for value in [65536, 65536, 65536, 65536] {
        u32le(&mut bytes, value);
    }
    for value in [[0, 0, 0], [10, 0, 0], [0, 0, 10]] {
        vec3(&mut bytes, value);
    }

    u16le(&mut bytes, 0);
    u16le(&mut bytes, 1);
    i16le(&mut bytes, 30);
    u16le(&mut bytes, 0);
    u32le(&mut bytes, 0);
    u32le(&mut bytes, 186);
    u16le(&mut bytes, 0);
    i16le(&mut bytes, 50);
    u16le(&mut bytes, 1);
    u16le(&mut bytes, 194);
    u32le(&mut bytes, 0);
    u32le(&mut bytes, 0);
    bytes
}

#[test]
fn bgi_parses_navigation_and_validates_cross_references() {
    let file = BgiFile::parse(&fixture(), "synthetic").unwrap();
    assert_eq!(file.triangles.len(), 1);
    assert_eq!(file.triangles[0].vertices, [0, 1, 2]);
    assert_eq!(file.triangles[0].neighbors, [-1, -1, -1]);
    assert_eq!(file.vertices[1], [10, 0, 0]);
    assert_eq!(file.floors[0].triangles, [0]);
    assert_eq!(file.animations[0].frame_values, [50]);
    assert_eq!(file.animations[0].triangles[0], [0]);
    assert_eq!(file.normals[0], [65536; 4]);
}

#[test]
fn bad_magic_sizes_and_invalid_indices_fail() {
    let mut bytes = fixture();
    bytes[0] = 0;
    assert!(BgiFile::parse(&bytes, "magic").is_err());

    let mut bytes = fixture();
    bytes[76..78].copy_from_slice(&99_i16.to_le_bytes());
    assert!(BgiFile::parse(&bytes, "vertex").is_err());

    let mut bytes = fixture();
    bytes.truncate(bytes.len() - 1);
    assert!(BgiFile::parse(&bytes, "size").is_err());
}
