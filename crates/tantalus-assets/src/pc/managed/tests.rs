use super::extract_resource;

#[test]
fn rejects_images_without_mz_signature() {
    let error = extract_resource(&[0; 64], "EmbeddedAsset/ratan_tbl.bin").unwrap_err();
    assert!(error.context.contains("MZ"));
}

#[test]
fn rejects_truncated_dos_header() {
    let error = extract_resource(&[0x4D, 0x5A], "any").unwrap_err();
    assert!(error.context.contains("end of file"));
}

#[test]
fn rejects_missing_pe_signature() {
    let mut bytes = vec![0u8; 0x40];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
    let error = extract_resource(&bytes, "any").unwrap_err();
    assert!(error.context.contains("PE signature") || error.context.contains("end of file"));
}
