use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tantalus_assets::pc::{CodecSupport, PcContainerKind, inspect_pc_container};
use tantalus_assets::*;
use tantalus_contracts::ContentId;
use tempfile::TempDir;

#[derive(Debug)]
struct MemorySource {
    entries: BTreeMap<String, Vec<u8>>,
    cancel_during_read: Option<CancellationToken>,
}

impl MemorySource {
    fn new(entries: &[(&str, &[u8])]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|(path, bytes)| ((*path).to_owned(), bytes.to_vec()))
                .collect(),
            cancel_during_read: None,
        }
    }
}

impl AssetSource for MemorySource {
    fn read(&self, request: &AssetRequest, max_bytes: u64) -> Result<Vec<u8>, ImportError> {
        if let Some(token) = &self.cancel_during_read {
            token.cancel();
        }
        let bytes = self
            .entries
            .get(&request.source_path)
            .ok_or_else(|| ImportError::invalid(&request.source_path, "resource is absent"))?
            .clone();
        if bytes.len() as u64 > max_bytes {
            return Err(ImportError::invalid(
                &request.source_path,
                "resource exceeds the file limit",
            ));
        }
        Ok(bytes)
    }
}

fn id(text: &str) -> ContentId {
    ContentId::parse(text).unwrap()
}

fn request(text: &str, path: &str) -> AssetRequest {
    AssetRequest {
        id: id(text),
        source_path: path.to_owned(),
        kind: AssetKind::Binary,
        codec: AssetCodec::Raw,
        dependencies: Vec::new(),
    }
}

fn plan(requests: Vec<AssetRequest>) -> ImportPlan {
    ImportPlan::new(requests)
}

#[test]
fn identical_inputs_produce_the_same_pack_in_any_request_order() {
    let event = request("fixture:event/opening", "event.bin");
    let mut field = request("fixture:field/opening", "field.bin");
    field.dependencies = vec![id("fixture:event/opening")];
    let source = MemorySource::new(&[("event.bin", b"event"), ("field.bin", b"field")]);
    let first_cache = TempDir::new().unwrap();
    let second_cache = TempDir::new().unwrap();

    let first = import_subset(
        &plan(vec![field.clone(), event.clone()]),
        &source,
        first_cache.path(),
        &CancellationToken::default(),
    )
    .unwrap();
    let second = import_subset(
        &plan(vec![event, field]),
        &source,
        second_cache.path(),
        &CancellationToken::default(),
    )
    .unwrap();

    assert_eq!(first.pack_id, second.pack_id);
    assert_eq!(first, second);
    assert_eq!(first.assets.len(), 2);
    assert_eq!(first.assets[0].id, id("fixture:event/opening"));
    assert_eq!(
        first.assets[1].dependencies,
        vec![id("fixture:event/opening")]
    );
    let store = CacheStore::new(first_cache.path());
    assert_eq!(store.load(&first.pack_id).unwrap(), first);
    assert_eq!(store.revisions().unwrap(), vec![first.pack_id]);
}

#[test]
fn directory_import_supports_non_ascii_paths_and_utf8() {
    let parent = TempDir::new().unwrap();
    let source_root = parent.path().join("ゲーム-データ");
    let cache = parent.path().join("キャッシュ");
    fs::create_dir_all(source_root.join("fields")).unwrap();
    fs::write(source_root.join("fields/field.txt"), "古代のフィールド").unwrap();
    let mut text = request("fixture:field/text", "fields/field.txt");
    text.codec = AssetCodec::Utf8;
    let source = DirectoryAssetSource::new(&source_root);

    let manifest = import_subset(
        &plan(vec![text]),
        &source,
        &cache,
        &CancellationToken::default(),
    )
    .unwrap();

    assert_eq!(manifest.assets[0].codec, AssetCodec::Utf8);
    assert_eq!(manifest.assets[0].source_path, "fields/field.txt");
    assert!(CacheStore::new(&cache).load(&manifest.pack_id).is_ok());
}

#[test]
fn traversal_and_absolute_source_paths_are_rejected() {
    let root = TempDir::new().unwrap();
    let source = DirectoryAssetSource::new(root.path());
    for path in ["../outside.bin", "/absolute.bin", ""] {
        let error = source
            .read(&request("fixture:binary/bad", path), 100)
            .unwrap_err();
        assert!(error.to_string().contains("relative"), "path: {path}");
    }
}

#[test]
fn limits_and_invalid_utf8_fail_without_a_complete_revision() {
    let cache = TempDir::new().unwrap();
    let source = MemorySource::new(&[("large.bin", b"12345"), ("text.txt", b"\xff")]);
    let mut limited = plan(vec![request("fixture:binary/large", "large.bin")]);
    limited.limits.max_file_bytes = 4;
    assert!(
        import_subset(
            &limited,
            &source,
            cache.path(),
            &CancellationToken::default()
        )
        .unwrap_err()
        .to_string()
        .contains("limit")
    );

    let mut text = request("fixture:text/invalid", "text.txt");
    text.codec = AssetCodec::Utf8;
    assert!(
        import_subset(
            &plan(vec![text]),
            &source,
            cache.path(),
            &CancellationToken::default()
        )
        .unwrap_err()
        .to_string()
        .contains("UTF-8")
    );
    assert!(
        CacheStore::new(cache.path())
            .revisions()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn dangling_duplicate_and_self_dependencies_are_specific_errors() {
    let cache = TempDir::new().unwrap();
    let source = MemorySource::new(&[("a.bin", b"a")]);
    let mut dangling = request("fixture:field/a", "a.bin");
    dangling.dependencies = vec![id("fixture:event/missing")];
    let error = import_subset(
        &plan(vec![dangling]),
        &source,
        cache.path(),
        &CancellationToken::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("dangling dependency"));

    let duplicate = plan(vec![
        request("fixture:binary/a", "a.bin"),
        request("fixture:binary/a", "a.bin"),
    ]);
    assert!(
        import_subset(
            &duplicate,
            &source,
            cache.path(),
            &CancellationToken::default()
        )
        .unwrap_err()
        .to_string()
        .contains("duplicate content identity")
    );

    let mut self_dependent = request("fixture:field/a", "a.bin");
    self_dependent.dependencies = vec![id("fixture:field/a")];
    assert!(
        import_subset(
            &plan(vec![self_dependent]),
            &source,
            cache.path(),
            &CancellationToken::default()
        )
        .unwrap_err()
        .to_string()
        .contains("depend on itself")
    );
    assert!(
        CacheStore::new(cache.path())
            .revisions()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cancellation_during_read_never_publishes_a_revision() {
    let cache = TempDir::new().unwrap();
    let token = CancellationToken::default();
    let mut source = MemorySource::new(&[("a.bin", b"a")]);
    source.cancel_during_read = Some(token.clone());

    let error = import_subset(
        &plan(vec![request("fixture:binary/a", "a.bin")]),
        &source,
        cache.path(),
        &token,
    )
    .unwrap_err();

    assert!(error.to_string().contains("cancelled"));
    assert!(
        CacheStore::new(cache.path())
            .revisions()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn failed_later_import_preserves_the_last_complete_revision() {
    let cache = TempDir::new().unwrap();
    let source = MemorySource::new(&[("a.bin", b"a")]);
    let valid = import_subset(
        &plan(vec![request("fixture:binary/a", "a.bin")]),
        &source,
        cache.path(),
        &CancellationToken::default(),
    )
    .unwrap();

    let mut invalid = request("fixture:binary/a", "a.bin");
    invalid.dependencies = vec![id("fixture:binary/missing")];
    assert!(
        import_subset(
            &plan(vec![invalid]),
            &source,
            cache.path(),
            &CancellationToken::default()
        )
        .is_err()
    );

    let store = CacheStore::new(cache.path());
    assert_eq!(store.revisions().unwrap(), vec![valid.pack_id.clone()]);
    assert_eq!(store.load(&valid.pack_id).unwrap(), valid);
}

#[test]
fn corrupted_cached_objects_are_detected_on_load() {
    let cache = TempDir::new().unwrap();
    let source = MemorySource::new(&[("a.bin", b"a")]);
    let manifest = import_subset(
        &plan(vec![request("fixture:binary/a", "a.bin")]),
        &source,
        cache.path(),
        &CancellationToken::default(),
    )
    .unwrap();
    let object = cache.path().join(&manifest.assets[0].object);
    fs::write(&object, b"corrupted").unwrap();

    let error = CacheStore::new(cache.path())
        .load(&manifest.pack_id)
        .unwrap_err();
    assert!(error.to_string().contains("does not match"));
    assert!(error.to_string().contains("fixture:binary/a"));
}

#[test]
fn unityfs_is_recognized_without_claiming_decode_support() {
    let fixture = TempDir::new().unwrap();
    let unity = fixture.path().join("p0data11.bin");
    fs::write(&unity, b"UnityFS\0synthetic").unwrap();
    let report = inspect_pc_container(&unity).unwrap();
    assert_eq!(report.kind, PcContainerKind::UnityFs);
    assert_eq!(report.codec_support, CodecSupport::Unsupported);
    assert!(report.detail.contains("not implemented"));

    let unknown = fixture.path().join("unknown.bin");
    fs::write(&unknown, b"UNKNOWN!").unwrap();
    assert_eq!(
        inspect_pc_container(&unknown).unwrap().kind,
        PcContainerKind::Unrecognized
    );

    let truncated = fixture.path().join("truncated.bin");
    fs::write(&truncated, b"Unity").unwrap();
    assert!(
        inspect_pc_container(&truncated)
            .unwrap_err()
            .to_string()
            .contains("truncated")
    );
}

#[test]
fn cache_root_failure_identifies_the_resource_and_operation() {
    let fixture = TempDir::new().unwrap();
    let cache_file = fixture.path().join("not-a-directory");
    fs::write(&cache_file, b"file").unwrap();
    let source = MemorySource::new(&[("a.bin", b"a")]);
    let error = import_subset(
        &plan(vec![request("fixture:binary/a", "a.bin")]),
        &source,
        &cache_file,
        &CancellationToken::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("create object cache"));
    assert!(error.to_string().contains("not-a-directory"));
}

#[test]
fn import_rejects_empty_and_unsupported_plans() {
    let cache = TempDir::new().unwrap();
    let source = MemorySource::new(&[]);
    assert!(
        import_subset(
            &plan(Vec::new()),
            &source,
            cache.path(),
            &CancellationToken::default()
        )
        .is_err()
    );

    let mut unsupported = plan(vec![request("fixture:binary/a", "a.bin")]);
    unsupported.target_schema = 99;
    let error = import_subset(
        &unsupported,
        &source,
        cache.path(),
        &CancellationToken::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("target schema 99"));
}

#[test]
fn source_missing_error_names_the_requested_path() {
    let cache = TempDir::new().unwrap();
    let source = MemorySource::new(&[]);
    let error = import_subset(
        &plan(vec![request("fixture:binary/missing", "missing.bin")]),
        &source,
        cache.path(),
        &CancellationToken::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("missing.bin"));
}

#[test]
fn object_path_is_relative_to_the_cache() {
    let cache = TempDir::new().unwrap();
    let source = MemorySource::new(&[("a.bin", b"a")]);
    let manifest = import_subset(
        &plan(vec![request("fixture:binary/a", "a.bin")]),
        &source,
        cache.path(),
        &CancellationToken::default(),
    )
    .unwrap();
    let object = Path::new(&manifest.assets[0].object);
    assert!(!object.is_absolute());
    assert!(cache.path().join(object).is_file());
}

#[test]
fn manifest_cannot_redirect_object_loading_outside_the_cache() {
    let cache = TempDir::new().unwrap();
    let source = MemorySource::new(&[("a.bin", b"a")]);
    let manifest = import_subset(
        &plan(vec![request("fixture:binary/a", "a.bin")]),
        &source,
        cache.path(),
        &CancellationToken::default(),
    )
    .unwrap();
    let manifest_path = cache
        .path()
        .join("revisions")
        .join(&manifest.pack_id)
        .join("manifest.json");
    let mut document: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    document["assets"][0]["object"] = serde_json::Value::String("../../outside.bin".to_owned());
    fs::write(&manifest_path, serde_json::to_vec(&document).unwrap()).unwrap();

    let error = CacheStore::new(cache.path())
        .load(&manifest.pack_id)
        .unwrap_err();
    assert!(error.to_string().contains("invalid object path"));
}
