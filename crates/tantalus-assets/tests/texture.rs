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
fn dxt5_alpha_uses_the_correct_interpolation_weight() {
    let mut block = solid_dxt5();
    block[0] = 200;
    block[1] = 100;
    block[2] = 2;
    let texture = Texture2D::parse(&fixture(4, 1, &block), "alpha").unwrap();
    assert_eq!(texture.rgba_bytes().unwrap()[3], 185);

    let mut block = solid_dxt5();
    block[0] = 100;
    block[1] = 200;
    block[2] = 2;
    let texture = Texture2D::parse(&fixture(4, 1, &block), "alpha").unwrap();
    assert_eq!(texture.rgba_bytes().unwrap()[3], 120);
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

#[test]
fn dxt1_texture_decodes_four_color_and_transparent_blocks() {
    let mut block = [0_u8; 8];
    // c0 <= c1 selects the 3-color mode: index 2 is the midpoint, 3 is transparent.
    block[0..2].copy_from_slice(&0x07e0_u16.to_le_bytes());
    block[2..4].copy_from_slice(&0xf800_u16.to_le_bytes());
    block[4..8].copy_from_slice(&(3_u32 | (2 << 2)).to_le_bytes());
    let mut bytes = fixture(4, 4, &block);
    bytes[24..28].copy_from_slice(&10_i32.to_le_bytes());
    let texture = Texture2D::parse(&bytes, "dxt1").unwrap();
    let rgba = texture.rgba_bytes().unwrap();
    // Decoded rows are flipped to top-first: block row 0 is the last row.
    assert_eq!(&rgba[48..52], [0, 0, 0, 0]);
    assert_eq!(&rgba[52..56], [127, 127, 0, 255]);
    assert_eq!(&rgba[56..60], [0, 255, 0, 255]);

    // The 4-color mode (c0 > c1) keeps every pixel opaque.
    block[0..2].copy_from_slice(&0xf800_u16.to_le_bytes());
    block[2..4].copy_from_slice(&0x07e0_u16.to_le_bytes());
    let mut bytes = fixture(4, 4, &block);
    bytes[24..28].copy_from_slice(&10_i32.to_le_bytes());
    let rgba = Texture2D::parse(&bytes, "dxt1")
        .unwrap()
        .rgba_bytes()
        .unwrap();
    assert!(rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
}

/// A fixture with another pixel format and mip count (`fixture` writes DXT5 and one level).
fn fixture_as(width: i32, height: i32, format: i32, mips: i32, data: &[u8]) -> Vec<u8> {
    let mut bytes = fixture(width, height, data);
    bytes[24..28].copy_from_slice(&format.to_le_bytes());
    bytes[28..32].copy_from_slice(&mips.to_le_bytes());
    bytes
}

#[test]
fn raw_formats_decode_with_unity_byte_orders() {
    // Two pixels each; rows are flipped, but a one-row texture is unchanged.
    let rgba32 = Texture2D::parse(&fixture_as(2, 1, 4, 1, &[1, 2, 3, 4, 5, 6, 7, 8]), "rgba32");
    assert_eq!(
        rgba32.unwrap().rgba_bytes().unwrap(),
        [1, 2, 3, 4, 5, 6, 7, 8]
    );
    // ARGB32 stores A, R, G, B.
    let argb32 = Texture2D::parse(&fixture_as(2, 1, 5, 1, &[4, 1, 2, 3, 8, 5, 6, 7]), "argb32");
    assert_eq!(
        argb32.unwrap().rgba_bytes().unwrap(),
        [1, 2, 3, 4, 5, 6, 7, 8]
    );
    // BGRA32 stores B, G, R, A.
    let bgra32 = Texture2D::parse(&fixture_as(1, 1, 14, 1, &[3, 2, 1, 9]), "bgra32");
    assert_eq!(bgra32.unwrap().rgba_bytes().unwrap(), [1, 2, 3, 9]);
    // RGB24 is opaque; Alpha8 is white with the stored alpha.
    let rgb24 = Texture2D::parse(&fixture_as(1, 1, 3, 1, &[10, 20, 30]), "rgb24");
    assert_eq!(rgb24.unwrap().rgba_bytes().unwrap(), [10, 20, 30, 255]);
    let alpha8 = Texture2D::parse(&fixture_as(2, 1, 1, 1, &[0, 77]), "alpha8");
    assert_eq!(
        alpha8.unwrap().rgba_bytes().unwrap(),
        [255, 255, 255, 0, 255, 255, 255, 77]
    );
    // RGB565 is a little-endian 16-bit value with red in the top five bits.
    let mut pixels = Vec::new();
    for value in [0xf800_u16, 0x07e0, 0x001f] {
        pixels.extend_from_slice(&value.to_le_bytes());
    }
    let rgb565 = Texture2D::parse(&fixture_as(3, 1, 7, 1, &pixels), "rgb565");
    assert_eq!(
        rgb565.unwrap().rgba_bytes().unwrap(),
        [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]
    );
}

#[test]
fn raw_format_size_must_match_the_declared_chain() {
    assert!(Texture2D::parse(&fixture_as(2, 1, 4, 1, &[0; 7]), "short").is_err());
    assert!(Texture2D::parse(&fixture_as(2, 2, 4, 1, &[0; 12]), "short").is_err());
    // Two levels of a 2x2 RGBA32: 16 + 4 bytes.
    assert!(Texture2D::parse(&fixture_as(2, 2, 4, 2, &[0; 20]), "chain").is_ok());
    assert!(Texture2D::parse(&fixture_as(2, 2, 4, 2, &[0; 16]), "chain").is_err());
}

#[test]
fn dxt_mip_chain_decodes_the_first_level_only() {
    // 8x8 DXT5 with four levels: 8x8 (4 blocks), 4x4, 2x2 and 1x1 (one block each).
    let mut data = Vec::new();
    for _ in 0..4 {
        data.extend_from_slice(&solid_dxt5());
    }
    let mut smaller = solid_dxt5();
    smaller[8..10].copy_from_slice(&0x001f_u16.to_le_bytes());
    smaller[10..12].copy_from_slice(&0x001f_u16.to_le_bytes());
    for _ in 0..3 {
        data.extend_from_slice(&smaller);
    }
    assert_eq!(data.len(), 112);
    let texture = Texture2D::parse(&fixture_as(8, 8, DXT5_FORMAT, 4, &data), "chain").unwrap();
    assert_eq!(texture.mip_count, 4);
    assert_eq!(texture.image_data.len(), 112);
    assert_eq!(texture.first_level_bytes(), 64);
    let rgba = texture.rgba_bytes().unwrap();
    assert_eq!(rgba.len(), 8 * 8 * 4);
    assert!(rgba.chunks_exact(4).all(|pixel| pixel == [255, 0, 0, 255]));
    // Level data that is not the whole chain is rejected.
    assert!(Texture2D::parse(&fixture_as(8, 8, DXT5_FORMAT, 4, &data[..64]), "cut").is_err());
    // A 1x1 chain tail needs a full block, so a 256x256 DXT1 chain of nine levels is 43,704 bytes.
    let total: usize = (0..9)
        .map(|level| {
            let side = (256_usize >> level).max(1);
            side.div_ceil(4) * side.div_ceil(4) * 8
        })
        .sum();
    assert_eq!(total, 43_704);
}
