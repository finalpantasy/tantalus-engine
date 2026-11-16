//! Field data the chain and the live client both load: the event-id
//! tables from the Memoria source, field dialog text, and walkmeshes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Parses `{id, "EVT_NAME"},` rows of Memoria's `FF9DBAll.EventDB`.
pub fn parse_event_db(source: &str) -> BTreeMap<i32, String> {
    let mut table = BTreeMap::new();
    for line in source.lines() {
        let Some(body) = line.trim().strip_prefix('{') else {
            continue;
        };
        let Some((id, rest)) = body.split_once(',') else {
            continue;
        };
        let Ok(id) = id.trim().parse::<i32>() else {
            continue;
        };
        let mut quoted = rest.split('"');
        let (Some(_), Some(name)) = (quoted.next(), quoted.next()) else {
            continue;
        };
        table.insert(id, name.to_owned());
    }
    table
}

/// Per-field text facts the scripted player needs.
#[derive(Default)]
pub struct FieldTextInfo {
    pub signals: BTreeMap<u16, Vec<i32>>,
    /// Text ids that contain a `[CHOO]` choice list.
    pub choices: BTreeSet<u16>,
    /// Literal text of each entry (tags dropped), for the trace record.
    pub plain: BTreeMap<u16, String>,
}

/// Field dialog text, for the `[SIGL=n]` signals windows raise.
///
/// Memoria loads `EmbeddedAsset/Text/<lang>/Field/<zone>.mes`, where the zone
/// is `EventEngineUtils.eventIDToMESID[fldMapNo]` (`HonoluluFieldMain`,
/// `FF9TextTool.UpdateFieldText`). The path resolves through the Unity
/// `ResourceManager` container in `mainData`. The map table is read at run
/// time from the pinned Memoria source, not copied here.
pub struct FieldText {
    data_dir: PathBuf,
    language: String,
    zones: BTreeMap<i32, i32>,
    /// Lower-case resource path -> (file name, path id).
    container: BTreeMap<String, (String, i64)>,
}

impl FieldText {
    pub fn open(data_dir: &Path, map_source: &str, language: String) -> Result<Self, String> {
        use tantalus_assets::pc::resource_container::{
            RESOURCE_MANAGER_CLASS_ID, parse_resource_container,
        };
        use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
        let main = data_dir.join("mainData");
        let mut file = StandaloneSerialized::open(&main).map_err(|error| error.to_string())?;
        let manager = file
            .objects()
            .iter()
            .find(|object| object.class_id == RESOURCE_MANAGER_CLASS_ID)
            .cloned()
            .ok_or("mainData has no ResourceManager")?;
        let bytes = file
            .read_object(&manager)
            .map_err(|error| error.to_string())?;
        let externals = file.inventory().external_paths.clone();
        let mut container = BTreeMap::new();
        for entry in parse_resource_container(&bytes, &main.display().to_string())
            .map_err(|error| error.to_string())?
        {
            let file_name = match usize::try_from(entry.file_id) {
                Ok(0) => "mainData".to_owned(),
                Ok(index) => match externals.get(index - 1) {
                    Some(path) => path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned(),
                    None => continue,
                },
                Err(_) => continue,
            };
            container.insert(entry.path.to_lowercase(), (file_name, entry.path_id));
        }
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            language,
            zones: parse_mes_map(map_source),
            container,
        })
    }

    /// `[SIGL]` values and choice texts for one field map, or why none were
    /// loaded.
    pub fn info(&self, map: i32) -> Result<FieldTextInfo, String> {
        use tantalus_assets::pc::mes::{MesToken, parse_mes_bytes};
        use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
        let zone = self
            .zones
            .get(&map)
            .ok_or_else(|| format!("map {map} has no eventIDToMESID entry"))?;
        let key = format!("embeddedasset/text/{}/field/{zone}", self.language);
        let (file_name, path_id) = self
            .container
            .get(&key)
            .or_else(|| self.container.get(&format!("{key}.mes")))
            .ok_or_else(|| format!("resource `{key}` not in the container"))?;
        let path = self.data_dir.join(file_name);
        let mut file = StandaloneSerialized::open(&path).map_err(|error| error.to_string())?;
        let object = file
            .objects()
            .iter()
            .find(|object| object.path_id == *path_id)
            .cloned()
            .ok_or_else(|| format!("{file_name} has no object {path_id}"))?;
        let asset = file
            .read_text_asset(&object)
            .map_err(|error| error.to_string())?;
        let mes = parse_mes_bytes(&asset.script, &key).map_err(|error| error.to_string())?;
        let mut info = FieldTextInfo::default();
        for entry in &mes.entries {
            let Ok(id) = u16::try_from(entry.id) else {
                continue;
            };
            let plain: String = entry
                .tokens
                .iter()
                .filter_map(|token| match token {
                    MesToken::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            info.plain.insert(id, plain);
            if entry
                .tokens
                .iter()
                .any(|token| matches!(token, MesToken::Tag(tag) if tag.code == "Choice"))
            {
                info.choices.insert(id);
            }
            let values: Vec<i32> = entry
                .tokens
                .iter()
                .filter_map(|token| match token {
                    MesToken::Tag(tag) if tag.code == "Signal" => tag.params.first()?.parse().ok(),
                    // `DialogBoxSymbols`: both increase tags raise
                    // `OnSignal(-1)`, which adds one to `gMesSignal`.
                    MesToken::Tag(tag)
                        if tag.code == "IncreaseSignal" || tag.code == "IncreaseSignalEx" =>
                    {
                        Some(-1)
                    }
                    _ => None,
                })
                .collect();
            if !values.is_empty() {
                info.signals.insert(id, values);
            }
        }
        Ok(info)
    }
}

/// `EventEngineUtils.eventIDToMESID` entries (`{map, zone},`), read from the
/// Memoria source after the dictionary's declaration.
pub fn parse_mes_map(source: &str) -> BTreeMap<i32, i32> {
    let Some(start) = source.find("eventIDToMESID") else {
        return BTreeMap::new();
    };
    let mut map = BTreeMap::new();
    for line in source[start..].lines().skip(1) {
        let line = line.trim();
        if line.starts_with("};") {
            break;
        }
        let Some(body) = line.strip_prefix('{') else {
            continue;
        };
        let mut parts = body.trim_end_matches(',').trim_end_matches('}').split(',');
        if let (Some(Ok(map_no)), Some(Ok(zone))) = (
            parts.next().map(|part| part.trim().parse()),
            parts.next().map(|part| part.trim().parse()),
        ) {
            map.insert(map_no, zone);
        }
    }
    map
}

/// `EventEngineUtils.eventIDToFBGID` entries (`{map, "FBG_..."},`), read
/// from the Memoria source after the dictionary's declaration.
pub fn parse_background_map(source: &str) -> BTreeMap<i32, String> {
    let Some(start) = source.find("eventIDToFBGID") else {
        return BTreeMap::new();
    };
    let mut map = BTreeMap::new();
    for line in source[start..].lines().skip(1) {
        let line = line.trim();
        if line.starts_with("};") {
            break;
        }
        let Some(body) = line.strip_prefix('{') else {
            continue;
        };
        let Some((id, rest)) = body.split_once(',') else {
            continue;
        };
        let Ok(id) = id.trim().parse::<i32>() else {
            continue;
        };
        if let Some(name) = rest.split('"').nth(1) {
            map.insert(id, name.to_owned());
        }
    }
    map
}

/// Finds the field bundle that registers `name` and loads its walkmesh.
pub fn load_navigation(
    bundles: &[PathBuf],
    name: &str,
) -> Result<tantalus_contracts::FieldNavigationMesh, String> {
    for bundle in bundles {
        if !bundle.exists() {
            continue;
        }
        match tantalus_assets::pc::bundled_field::load_field_navigation(bundle, name) {
            Ok(Some(mesh)) => return Ok(mesh),
            Ok(None) => {}
            Err(error) => return Err(format!("{}: {error}", bundle.display())),
        }
    }
    Err(format!("no field bundle registers `{name}`"))
}

/// The field's tile animations in their post-load state and its camera
/// count, from the first bundle that registers `name`.
pub fn load_tile_animations(
    bundles: &[PathBuf],
    name: &str,
) -> Result<(Vec<tantalus_game::field::tile_animation::TileAnimation>, u8), String> {
    for bundle in bundles {
        if !bundle.exists() {
            continue;
        }
        match tantalus_assets::pc::bundled_field::load_field_animations(bundle, name) {
            Ok(Some((animations, cameras))) => {
                let animations = animations
                    .into_iter()
                    .map(|animation| {
                        tantalus_game::field::tile_animation::TileAnimation::from_file(
                            animation.camera,
                            animation.frames,
                        )
                    })
                    .collect();
                return Ok((animations, u8::try_from(cameras).unwrap_or(u8::MAX)));
            }
            Ok(None) => {}
            Err(error) => return Err(format!("{}: {error}", bundle.display())),
        }
    }
    Err(format!("no field bundle registers `{name}`"))
}
