//! Which language copy of a per-language asset to use.
//!
//! The game keeps one copy of every event script per language
//! (`.../eventbinary/<field|battle|world>/<language>/<name>.eb.bytes`, language `us`, `uk`, `jp`,
//! `es`, `fr`, `gr`, `it`), all under the same TextAsset name, and the text tables
//! (`embeddedasset/text/<language>/...`) follow the same language. The reference picks the copy
//! by `Localization.CurrentSymbol`: `EventEngineUtils.loadEventData` and `loadEventAsScript` build
//! `EB_FILE_PATH + subfolder + CurrentSymbol + "/" + name`. Taking the first asset with the name
//! would run, for example, the Spanish script against the English text table (text ids are not
//! the same across languages), so every lookup goes through this module.

use crate::pc::asset_bundle::AssetBundleMetadata;
use crate::pc::serialized::SerializedFile;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

/// The language every tool starts with: English, as the text tables are.
pub const DEFAULT_LANGUAGE: &str = "us";

static LANGUAGE: RwLock<Option<String>> = RwLock::new(None);

/// The language lookups use now (lower case): the one set by [`set_asset_language`], else
/// [`DEFAULT_LANGUAGE`].
pub fn asset_language() -> String {
    LANGUAGE
        .read()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(|| DEFAULT_LANGUAGE.to_owned())
}

/// Sets the language for later lookups (`us`, `uk`, `jp`, `es`, `fr`, `gr`, `it`, any case). Set
/// it to the language the text tables are opened with.
pub fn set_asset_language(language: &str) {
    if let Ok(mut guard) = LANGUAGE.write() {
        *guard = Some(language.to_ascii_lowercase());
    }
}

/// `(kind, language, file)` of an event script container path such as
/// `assets/resources/commonasset/eventengine/eventbinary/field/us/evt_a_1.eb.bytes`.
fn event_script_path(path: &str) -> Option<(&str, &str, &str)> {
    let rest = path.split("/eventbinary/").nth(1)?;
    let mut parts = rest.splitn(3, '/');
    Some((parts.next()?, parts.next()?, parts.next()?))
}

/// The path ids of the event scripts a bundle holds in a language other than `language`. For a
/// script missing in `language`, the `us` copy (else the first one) is kept, so a language with
/// fewer scripts still loads. Callers skip these objects when scanning for a TextAsset by name.
pub fn other_language_event_scripts(
    serialized: &SerializedFile<'_>,
    source: &str,
    language: &str,
) -> BTreeSet<i64> {
    let report = serialized.report();
    let Some(metadata) = report
        .objects
        .iter()
        .position(|object| object.class_id == 142)
        .and_then(|index| serialized.object_data(index).ok())
        .and_then(|data| AssetBundleMetadata::parse(data, source).ok())
    else {
        return BTreeSet::new();
    };
    let language = language.to_ascii_lowercase();
    // (kind, file) -> language -> path id
    let mut copies: BTreeMap<(&str, &str), BTreeMap<&str, i64>> = BTreeMap::new();
    for (path, info) in &metadata.container {
        if let Some((kind, copy_language, file)) = event_script_path(path) {
            copies
                .entry((kind, file))
                .or_default()
                .insert(copy_language, info.path_id);
        }
    }
    let mut skipped = BTreeSet::new();
    for by_language in copies.values() {
        let keep = by_language
            .get(language.as_str())
            .or_else(|| by_language.get(DEFAULT_LANGUAGE))
            .or_else(|| by_language.values().next())
            .copied();
        skipped.extend(by_language.values().copied().filter(|id| Some(*id) != keep));
    }
    skipped
}
