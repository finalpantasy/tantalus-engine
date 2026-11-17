//! Opens a Unity raw bundle's first entry as a serialized store without
//! copying it: the bundles are hundreds of megabytes and stored
//! uncompressed, so the file is mapped and parsed in place.

use std::path::Path;

use tantalus_assets::pc::model_assembly::SerializedStore;
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::unity_raw::UnityRawBundle;

/// The most one entry may take.
const MAX_BUNDLE_BYTES: usize = 600 * 1024 * 1024;

pub fn with_store<T>(
    path: &Path,
    run: impl FnOnce(&SerializedStore<'_, '_>) -> Result<T, String>,
) -> Result<T, String> {
    let source = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bundle = UnityRawBundle::open(path).map_err(|error| error.to_string())?;
    let bytes = bundle
        .map_entry(0, MAX_BUNDLE_BYTES)
        .map_err(|error| error.to_string())?;
    let file = SerializedFile::parse(&bytes, &source).map_err(|error| error.to_string())?;
    let store = SerializedStore::new(&file, &source).map_err(|error| error.to_string())?;
    run(&store)
}
