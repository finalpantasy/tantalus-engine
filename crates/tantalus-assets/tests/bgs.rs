use tantalus_assets::pc::bgs::BgsFile;

fn u16le(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn i16le(bytes: &mut Vec<u8>, value: i16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn u32le(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = Vec::new();
    u16le(&mut bytes, 202);
    u16le(&mut bytes, 1);
    for _ in 0..4 {
        u16le(&mut bytes, 1);
    }
    for offset in [52, 68, 124, 136] {
        u32le(&mut bytes, offset);
    }
    for _ in 0..12 {
        i16le(&mut bytes, 0);
    }

    u32le(&mut bytes, (1 << 8) | 2);
    u32le(&mut bytes, 0);
    i16le(&mut bytes, 256);
    u16le(&mut bytes, 0);
    u32le(&mut bytes, 200);

    u32le(&mut bytes, 2 | (3 << 8) | (4 << 20));
    u16le(&mut bytes, 16);
    u16le(&mut bytes, 16);
    for _ in 0..14 {
        i16le(&mut bytes, 0);
    }
    u32le(&mut bytes, 1 << 16);
    u32le(&mut bytes, 196);
    u32le(&mut bytes, 188);
    u32le(&mut bytes, 0);
    u32le(&mut bytes, 0);

    for value in [1, 2, 3] {
        i16le(&mut bytes, value);
    }
    u16le(&mut bytes, 0);
    bytes.extend_from_slice(&[4, 5, 6, 0]);

    u16le(&mut bytes, 100);
    for row in [[4096, 0, 0], [0, 4096, 0], [0, 0, 4096]] {
        for value in row {
            i16le(&mut bytes, value);
        }
    }
    for _ in 0..3 {
        u32le(&mut bytes, 0);
    }
    for _ in 0..2 {
        i16le(&mut bytes, 0);
    }
    i16le(&mut bytes, 320);
    i16le(&mut bytes, 224);
    for _ in 0..4 {
        i16le(&mut bytes, 0);
    }
    u32le(&mut bytes, 0);

    u32le(&mut bytes, 0);
    u32le(&mut bytes, (16 << 8) | (16 << 18));
    u32le(&mut bytes, 7 | (8 << 12) | (9 << 22));
    bytes.extend_from_slice(&[1, 0xfe]);
    bytes
}

#[test]
fn bgs_parses_scene_camera_overlay_sprite_light_and_animation() {
    let file = BgsFile::parse(&fixture(), "synthetic").unwrap();
    assert_eq!(file.scene_length, 202);
    assert_eq!(file.cameras[0].projection, 100);
    assert_eq!(file.cameras[0].rotation[0][0], 4096);
    assert_eq!(file.overlays[0].current_z, 3);
    assert_eq!(file.overlays[0].original_z, 4);
    assert_eq!(file.overlays[0].sprites[0].width, 16);
    assert_eq!(file.overlays[0].sprites[0].depth, 7);
    assert_eq!(file.overlays[0].sprites[0].offset_x, 9);
    assert_eq!(file.lights[0].color, [4, 5, 6]);
    assert_eq!(file.animations[0].frames, [(1, -2)]);
}

#[test]
fn invalid_scene_length_and_camera_reference_fail() {
    let mut bytes = fixture();
    bytes[0..2].copy_from_slice(&1_u16.to_le_bytes());
    assert!(BgsFile::parse(&bytes, "length").is_err());

    let mut bytes = fixture();
    bytes[104] = 1;
    assert!(BgsFile::parse(&bytes, "camera").is_err());
}
