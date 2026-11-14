use tantalus_assets::pc::texture::{DXT5_FORMAT, Texture2D};

fn i32le(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn aligned(bytes: &mut Vec<u8>) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
}

fn solid_dxt5() -> [u8; 16] {
    let mut block = [0; 16];
    block[0] = 255;
    block[8..10].copy_from_slice(&0xf800_u16.to_le_bytes());
    block[10..12].copy_from_slice(&0x07e0_u16.to_le_bytes());
    block
}

fn fixture(width: i32, height: i32, pixels: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    i32le(&mut bytes, 5);
    bytes.extend_from_slice(b"atlas");
    aligned(&mut bytes);
    for value in [width, height, pixels.len() as i32, DXT5_FORMAT, 1] {
        i32le(&mut bytes, value);
    }
    bytes.extend_from_slice(&[1, 1]);
    aligned(&mut bytes);
    for value in [1, 2, 1, 0] {
        i32le(&mut bytes, value);
    }
    bytes.extend_from_slice(&0_f32.to_le_bytes());
    for value in [1, 0, 0] {
        i32le(&mut bytes, value);
    }
    i32le(&mut bytes, pixels.len() as i32);
    bytes.extend_from_slice(pixels);
    aligned(&mut bytes);
    bytes
}

#[test]
fn dxt5_texture_preserves_metadata_and_decodes_solid_pixels() {
    let texture = Texture2D::parse(&fixture(4, 1, &solid_dxt5()), "synthetic").unwrap();
    assert_eq!(texture.name, "atlas");
    assert_eq!(texture.width, 4);
    assert_eq!(texture.height, 1);
    assert!(texture.is_readable && texture.read_allowed);
    let rgba = texture.rgba_bytes().unwrap();
    assert_eq!(rgba.len(), 16);
    assert!(rgba.chunks_exact(4).all(|pixel| pixel == [255, 0, 0, 255]));
}

#[test]
fn invalid_dimensions_sizes_formats_and_truncation_fail() {
    assert!(Texture2D::parse(&fixture(0, 1, &[]), "dimensions").is_err());
    assert!(Texture2D::parse(&fixture(4, 5, &solid_dxt5()), "size").is_err());
    let mut unsupported = fixture(4, 1, &solid_dxt5());
    unsupported[24..28].copy_from_slice(&13_i32.to_le_bytes());
    assert!(Texture2D::parse(&unsupported, "format").is_err());
    assert!(Texture2D::parse(&fixture(4, 1, &solid_dxt5()[..15]), "truncated").is_err());
}
