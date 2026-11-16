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
    /// How each entry lays out as a dialog window.
    pub layouts: BTreeMap<u16, TextLayout>,
    /// The parsed entries, for the dialog renderer.
    pub entries: BTreeMap<u16, tantalus_assets::pc::mes::MesEntry>,
}

/// A dialog text reduced to what a window draws: its rows (control codes
/// interpreted or dropped), the size and position its tags ask for and where
/// its choice list starts. Only the first page is kept (`[PAGE]` ends it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextLayout {
    pub rows: Vec<String>,
    /// `[STRT=width,lines]`: window width in PSX pixels and text line count.
    pub size: Option<(i32, i32)>,
    /// `[MPOS=x,y]`: fixed window position.
    pub position: Option<(i32, i32)>,
    /// First row of the `[CHOO]` list; the rows after it are the answers.
    pub choice_row: Option<usize>,
}

/// Default name a party or character tag stands for in a dialog window.
fn tag_name(keyword: &str) -> &'static str {
    match keyword {
        "ZDNE" | "1" => "Zidane",
        "VIVI" | "2" => "Vivi",
        "DGGR" | "3" => "Dagger",
        "STNR" | "4" => "Steiner",
        "FRYA" | "5" => "Freya",
        "QUIN" | "6" => "Quina",
        "EIKO" | "7" => "Eiko",
        "AMRT" | "8" => "Amarant",
        _ => "Zidane",
    }
}

/// Reads a text entry's tokens into a window layout. Memoria:
/// `Dialog.Phrase`/`TextParser` (`[STRT]`, `[MPOS]`, `[CHOO]`, `[PAGE]`).
pub fn text_layout(tokens: &[tantalus_assets::pc::mes::MesToken]) -> TextLayout {
    use tantalus_assets::pc::mes::MesToken;
    let mut layout = TextLayout::default();
    let mut rows = vec![String::new()];
    let pair = |params: &[String]| -> Option<(i32, i32)> {
        Some((params.first()?.parse().ok()?, params.get(1)?.parse().ok()?))
    };
    for token in tokens {
        match token {
            MesToken::Text { text } => {
                for (index, part) in text.split('\n').enumerate() {
                    if index > 0 {
                        rows.push(String::new());
                    }
                    if let Some(row) = rows.last_mut() {
                        row.push_str(part);
                    }
                }
            }
            MesToken::Tag(tag) => match tag.code.as_str() {
                "DialogSize" => layout.size = pair(&tag.params),
                "Position" => layout.position = pair(&tag.params),
                "Choice" => layout.choice_row = Some(rows.len() - 1),
                "NewPage" => break,
                "CharacterName" | "Party" => {
                    let name = tag_name(tag.params.first().map_or("", String::as_str));
                    if let Some(row) = rows.last_mut() {
                        row.push_str(name);
                    }
                }
                "Variable" => {
                    if let Some(row) = rows.last_mut() {
                        row.push('0');
                    }
                }
                "Item" => {
                    if let Some(row) = rows.last_mut() {
                        row.push_str("Item");
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    let mut choice_row = layout.choice_row;
    let mut kept = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let row = row.split_whitespace().collect::<Vec<_>>().join(" ");
        if row.is_empty() {
            // Blank rows before the text or choices are layout only.
            if choice_row.is_some_and(|start| index < start) {
                choice_row = choice_row.map(|start| start - 1);
            }
            continue;
        }
        kept.push(row);
    }
    layout.rows = kept;
    layout.choice_row = choice_row;
    layout
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

    /// Reads and parses the `.mes` resource `key` from the Unity container.
    fn read_mes(&self, key: &str) -> Result<tantalus_assets::pc::mes::MesFile, String> {
        use tantalus_assets::pc::mes::parse_mes_bytes;
        use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
        let (file_name, path_id) = self
            .container
            .get(key)
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
        parse_mes_bytes(&asset.script, key).map_err(|error| error.to_string())
    }

    /// Item names by item id (`ItemImporter`: `Item/itm_name.mes`), for the
    /// `[ITEM]` tag.
    pub fn item_names(&self) -> Result<BTreeMap<i32, String>, String> {
        self.names("item/itm_name")
    }

    /// Plain text of every entry of a text asset under
    /// `embeddedasset/text/<language>/`, by entry id (for example
    /// `ability/aa_name`).
    pub fn names(&self, asset: &str) -> Result<BTreeMap<i32, String>, String> {
        use tantalus_assets::pc::mes::MesToken;
        let mes = self.read_mes(&format!("embeddedasset/text/{}/{asset}", self.language))?;
        Ok(mes
            .entries
            .iter()
            .map(|entry| {
                let name: String = entry
                    .tokens
                    .iter()
                    .filter_map(|token| match token {
                        MesToken::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                (entry.id, name.trim().to_owned())
            })
            .collect())
    }

    /// `[SIGL]` values and choice texts for one field map, or why none were
    /// loaded.
    pub fn info(&self, map: i32) -> Result<FieldTextInfo, String> {
        use tantalus_assets::pc::mes::MesToken;
        let zone = self
            .zones
            .get(&map)
            .ok_or_else(|| format!("map {map} has no eventIDToMESID entry"))?;
        let key = format!("embeddedasset/text/{}/field/{zone}", self.language);
        let mes = self.read_mes(&key)?;
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
            info.layouts.insert(id, text_layout(&entry.tokens));
            info.entries.insert(id, entry.clone());
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

#[cfg(test)]
mod layout_tests {
    use super::*;
    use tantalus_assets::pc::mes::tokenize;

    #[test]
    fn layout_reads_choices_size_position_and_names() {
        let tokens =
            tokenize("[STRT=120,3][MPOS=40,60][ZDNE]: Ready?\n[CHOO=0,0]Yes\nNo[PAGE]later");
        let layout = text_layout(&tokens);
        assert_eq!(layout.size, Some((120, 3)));
        assert_eq!(layout.position, Some((40, 60)));
        assert_eq!(layout.rows, ["Zidane: Ready?", "Yes", "No"]);
        assert_eq!(layout.choice_row, Some(1));
    }

    #[test]
    fn layout_drops_codes_it_does_not_draw() {
        let layout = text_layout(&tokenize("[IMME][C8B040]Hi[TIME=1] there[WAIT=30]"));
        assert_eq!(layout.rows, ["Hi there"]);
        assert_eq!(layout.choice_row, None);
    }
}
