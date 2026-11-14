use tantalus_assets::pc::text_asset::TextAsset;

fn string(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as i32).to_le_bytes());
    bytes.extend_from_slice(value);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
}

#[test]
fn text_asset_preserves_raw_script_bytes_and_alignment() {
    let mut bytes = Vec::new();
    string(&mut bytes, "fixture.bgs".as_bytes());
    string(&mut bytes, &[0, 1, 0xff, 2, 3]);
    string(&mut bytes, "field/path".as_bytes());
    let asset = TextAsset::parse(&bytes, "synthetic").unwrap();
    assert_eq!(asset.name, "fixture.bgs");
    assert_eq!(asset.script, [0, 1, 0xff, 2, 3]);
    assert_eq!(asset.path_name, "field/path");
}

#[test]
fn invalid_utf8_truncation_and_trailing_data_fail() {
    let mut bytes = Vec::new();
    string(&mut bytes, &[0xff]);
    string(&mut bytes, b"data");
    string(&mut bytes, b"");
    assert!(TextAsset::parse(&bytes, "utf8").is_err());

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&100_i32.to_le_bytes());
    bytes.push(1);
    assert!(TextAsset::parse(&bytes, "truncated").is_err());

    let mut bytes = Vec::new();
    string(&mut bytes, b"asset");
    string(&mut bytes, b"data");
    string(&mut bytes, b"");
    bytes.push(1);
    assert!(TextAsset::parse(&bytes, "trailing").is_err());
}
