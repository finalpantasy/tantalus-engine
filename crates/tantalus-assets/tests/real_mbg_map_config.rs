//! Real-install check for the mbgdata and mapconfigdata readers; skipped
//! when the Steam install is not present. Override the bundle location with
//! `TANTALUS_FFIX_P0DATA7`.

use std::path::PathBuf;
use tantalus_assets::pc::asset_bundle::AssetBundleMetadata;
use tantalus_assets::pc::map_config::MapConfig;
use tantalus_assets::pc::mbg::MbgFile;
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::text_asset::TextAsset;
use tantalus_assets::pc::unity_raw::UnityRawBundle;

fn bundle_path() -> Option<PathBuf> {
    let path = std::env::var_os("TANTALUS_FFIX_P0DATA7")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                "E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/StreamingAssets/p0data7.bin",
            )
        });
    path.is_file().then_some(path)
}

#[test]
fn real_mbg_and_map_config_parse_completely() {
    let Some(path) = bundle_path() else {
        eprintln!("skipping: p0data7.bin not found");
        return;
    };
    let bundle = UnityRawBundle::open(&path).unwrap();
    let bytes = bundle.read_entry(0, 600 * 1024 * 1024).unwrap();
    let source = path.to_string_lossy();
    let serialized = SerializedFile::parse(&bytes, &source).unwrap();
    let report = serialized.report();
    let container = report
        .objects
        .iter()
        .position(|object| object.class_id == 142)
        .unwrap();
    let metadata =
        AssetBundleMetadata::parse(serialized.object_data(container).unwrap(), &source).unwrap();
    let (mut mbg, mut config) = (0, 0);
    for (asset_path, info) in &metadata.container {
        let index = report
            .objects
            .iter()
            .position(|object| object.path_id == info.path_id)
            .unwrap();
        if asset_path.ends_with(".mbgdata.bytes") {
            let asset =
                TextAsset::parse(serialized.object_data(index).unwrap(), asset_path).unwrap();
            let file = MbgFile::parse(&asset.script, asset_path).unwrap();
            assert!(file.trailing.is_empty(), "{asset_path}");
            assert_eq!(
                file.header_bytes() + file.unconsumed_bytes(),
                asset.script.len()
            );
            mbg += 1;
        } else if asset_path.contains("/mapconfigdata/") {
            let asset =
                TextAsset::parse(serialized.object_data(index).unwrap(), asset_path).unwrap();
            let parsed = MapConfig::parse(&asset.script, asset_path).unwrap();
            assert_eq!(
                parsed.consumed_bytes() + parsed.trailing.len(),
                asset.script.len()
            );
            config += 1;
        }
    }
    assert_eq!(mbg, 56);
    assert_eq!(config, 818);
}

#[test]
fn real_fmv005_has_a_measurable_frame_count() {
    let Some(root) = bundle_path().and_then(|path| {
        path.parent()
            .and_then(|parent| parent.parent())
            .map(|install| install.to_path_buf())
    }) else {
        eprintln!("skipping: install root not found");
        return;
    };
    let movie = root.join("StreamingAssets/ma/FMV005.bytes");
    if !movie.is_file() {
        eprintln!("skipping: FMV005.bytes not found");
        return;
    }
    let report = tantalus_assets::pc::ogg::scan_ogg_file(&movie, false).unwrap();
    let theora = report
        .streams
        .iter()
        .find(|stream| stream.codec.name() == "theora")
        .expect("theora stream");
    let frames = theora.frame_count.expect("frame count");
    assert!(
        frames > 100,
        "FMV005 should run seconds, got {frames} frames"
    );
    eprintln!("FMV005: {frames} frames, {:?}", theora.duration_seconds);
}
