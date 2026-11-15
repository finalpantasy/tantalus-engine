use std::path::PathBuf;
use tantalus_assets::pc::model_assembly::pose::Interpolation;
use tantalus_assets::pc::model_assembly::{
    AssembleOptions, ModelCatalog, SerializedStore, assemble_all, assemble_model, clip_entries,
    clip_report, embedded_clip_report,
};
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::unity_raw::UnityRawBundle;

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
fn real_install_assembles_every_model_and_binds_clips() {
    let (Some(models_path), Some(clips_path)) =
        (install_bundle("p0data4.bin"), install_bundle("p0data5.bin"))
    else {
        return;
    };
    let models_bytes = UnityRawBundle::open(&models_path)
        .unwrap()
        .read_entry(0, 600 * 1024 * 1024)
        .unwrap();
    let models_file = SerializedFile::parse(&models_bytes, "p0data4").unwrap();
    let store = SerializedStore::new(&models_file, "p0data4").unwrap();
    let catalog = ModelCatalog::from_container(&store.container().unwrap());
    assert_eq!(catalog.dirs.len(), 525);

    // One known model end to end.
    let zidane = catalog.find("2/98").unwrap();
    let model = assemble_model(
        &store,
        &zidane.label,
        zidane.roots[0],
        AssembleOptions::default(),
    )
    .unwrap();
    assert_eq!(model.nodes.len(), 33);
    assert_eq!(model.parts.len(), 4);
    assert_eq!(model.issues.total(), 0);
    assert!(model.rest_skin_error < 1e-2);
    assert_eq!(model.summary().textures_decoded, 2);

    let (report, models) = assemble_all(
        &store,
        &catalog,
        AssembleOptions {
            decode_textures: false,
        },
    );
    assert_eq!(report.assembled, 522);
    assert_eq!(report.directories_without_root, 3);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(report.skinned_parts, 1_586);
    assert_eq!(report.nodes, 16_645);
    assert_eq!(report.bone_resolution_failures, 0);
    assert_eq!(report.bone_count_mismatches, 0);
    assert_eq!(report.textures_undecodable, 2);

    let embedded = embedded_clip_report(&store, &models, Interpolation::Hermite);
    assert_eq!(embedded.clips, 105);
    assert_eq!(embedded.fully_matched, 105);
    assert_eq!(embedded.t0_equals_prefab, 105);

    let clip_bytes = UnityRawBundle::open(&clips_path)
        .unwrap()
        .read_entry(0, 600 * 1024 * 1024)
        .unwrap();
    let clip_file = SerializedFile::parse(&clip_bytes, "p0data5").unwrap();
    let clip_store = SerializedStore::new(&clip_file, "p0data5").unwrap();
    let (clips, other) = clip_entries(&clip_store.container().unwrap());
    assert_eq!((clips.len(), other.len()), (9_278, 0));
    let clips_report = clip_report(
        &clip_store,
        &clips,
        &catalog,
        &models,
        Interpolation::Hermite,
    );
    assert_eq!(clips_report.decode_failed, 0);
    assert!(
        clips_report.paired_fully_matched >= 7_900,
        "{clips_report:?}"
    );
    assert!(clips_report.any_skeleton_fully_matched >= 9_100);
    assert!(clips_report.bounds_checked >= 7_900);
}
