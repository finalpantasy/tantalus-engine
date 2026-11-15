use tantalus_assets::pc::asset_bundle::AssetBundleMetadata;

fn i32le(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn string(bytes: &mut Vec<u8>, value: &str) {
    i32le(bytes, value.len() as i32);
    bytes.extend_from_slice(value.as_bytes());
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
}

fn info(bytes: &mut Vec<u8>, path_id: i64) {
    i32le(bytes, 1);
    i32le(bytes, 1);
    i32le(bytes, 0);
    bytes.extend_from_slice(&path_id.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = Vec::new();
    string(&mut bytes, "");
    i32le(&mut bytes, 1);
    i32le(&mut bytes, 0);
    bytes.extend_from_slice(&10_i64.to_le_bytes());
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    i32le(&mut bytes, 1);
    string(
        &mut bytes,
        "assets/resources/fieldmaps/fbg_n00_tshp_map001_th_cgr_0/atlas.png",
    );
    info(&mut bytes, -7866888658012748271);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    info(&mut bytes, 0);
    i32le(&mut bytes, 1);
    string(&mut bytes, "bundle");
    i32le(&mut bytes, 1);
    string(&mut bytes, "dependency.bin");
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

#[test]
fn asset_bundle_container_resolves_the_field_atlas_pointer() {
    let metadata = AssetBundleMetadata::parse(&fixture(), "synthetic").unwrap();
    assert_eq!(metadata.preload_table, [(0, 10)]);
    assert_eq!(metadata.container.len(), 1);
    assert_eq!(metadata.asset_bundle_name, "bundle");
    assert_eq!(metadata.dependencies, ["dependency.bin"]);
    assert_eq!(
        metadata
            .atlas_for("FBG_N00_TSHP_MAP001_TH_CGR_0")
            .unwrap()
            .path_id,
        -7866888658012748271
    );
    assert!(metadata.atlas_for("missing").is_none());
}

#[test]
fn malformed_asset_bundle_metadata_is_rejected() {
    let mut bytes = fixture();
    bytes.truncate(bytes.len() - 4);
    assert!(AssetBundleMetadata::parse(&bytes, "truncated").is_err());
    assert!(AssetBundleMetadata::parse(&[0xff; 16], "invalid").is_err());
}
