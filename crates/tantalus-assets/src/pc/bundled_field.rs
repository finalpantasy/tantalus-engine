use crate::install::ImportError;
use crate::pc::asset_bundle::AssetBundleMetadata;
use crate::pc::bgi::BgiFile;
use crate::pc::bgs::BgsFile;
use crate::pc::field::FieldMapper;
use crate::pc::serialized::SerializedFile;
use crate::pc::text_asset::TextAsset;
use crate::pc::texture::Texture2D;
use crate::pc::unity_raw::UnityRawBundle;
use std::path::Path;
use tantalus_contracts::{ContentId, FieldSceneDescription};

pub struct BundledField {
    pub name: String,
    pub scene: FieldSceneDescription,
    pub scene_offset: [i16; 2],
    pub atlas_id: ContentId,
    pub atlas_path_id: i64,
    pub atlas: Texture2D,
}

impl BundledField {
    pub fn load(bundle_path: &Path, field_name: &str) -> Result<Self, ImportError> {
        let source = bundle_path.display().to_string();
        let bundle = UnityRawBundle::open(bundle_path)?;
        let bytes = bundle.read_entry(0, 300 * 1024 * 1024)?;
        let serialized = SerializedFile::parse(&bytes, &source)?;

        let metadata_index = serialized
            .report()
            .objects
            .iter()
            .position(|object| object.class_id == 142)
            .ok_or_else(|| ImportError::invalid(&source, "AssetBundle metadata is missing"))?;
        let metadata = AssetBundleMetadata::parse(
            serialized.object_data(metadata_index)?,
            &format!("{source}:AssetBundle"),
        )?;
        let atlas_info = metadata.atlas_for(field_name).cloned().ok_or_else(|| {
            ImportError::invalid(
                &source,
                format!("no atlas is registered for field `{field_name}`"),
            )
        })?;

        let scene_name = format!("{field_name}.bgs");
        let navigation_name = format!("{field_name}.bgi");
        let scene_asset = load_text_asset(&serialized, &scene_name, &source)?;
        let navigation_asset = load_text_asset(&serialized, &navigation_name, &source)?;
        let scene = BgsFile::parse(&scene_asset.script, &scene_name)?;
        let navigation = BgiFile::parse(&navigation_asset.script, &navigation_name)?;
        let atlas_id = ContentId::parse(&format!("ffix:field/{}/atlas", field_name.to_lowercase()))
            .map_err(|error| ImportError::invalid(&source, error.to_string()))?;
        let atlas_index = serialized
            .report()
            .objects
            .iter()
            .position(|object| object.class_id == 28 && object.path_id == atlas_info.path_id)
            .ok_or_else(|| {
                ImportError::invalid(
                    &source,
                    format!(
                        "field `{field_name}` atlas path ID {} is not a Texture2D in this bundle",
                        atlas_info.path_id
                    ),
                )
            })?;
        let atlas = Texture2D::parse(
            serialized.object_data(atlas_index)?,
            &format!("{source}:{}", atlas_info.path_id),
        )?;
        let mapped = FieldMapper::map_upscaled(
            &scene,
            &navigation,
            atlas_id.clone(),
            0,
            [atlas.width as u32, atlas.height as u32],
            32,
            2,
        )?;

        Ok(Self {
            name: field_name.to_owned(),
            scene: mapped,
            scene_offset: scene.current,
            atlas_id,
            atlas_path_id: atlas_info.path_id,
            atlas,
        })
    }
}

fn load_text_asset(
    serialized: &SerializedFile<'_>,
    name: &str,
    source: &str,
) -> Result<TextAsset, ImportError> {
    for (index, object) in serialized.report().objects.iter().enumerate() {
        if object.class_id != 49 {
            continue;
        }
        let asset = TextAsset::parse(serialized.object_data(index)?, name)?;
        if asset.name == name {
            return Ok(asset);
        }
    }
    Err(ImportError::invalid(
        source,
        format!("field TextAsset `{name}` was not found"),
    ))
}

/// Loads only the walkmesh (`<field>.bgi`) of one field from a field bundle
/// (`p0data11.bin`-`p0data19.bin`). Returns `Ok(None)` when this bundle does
/// not register the field, so callers can try the next bundle.
pub fn load_field_navigation(
    bundle_path: &Path,
    field_name: &str,
) -> Result<Option<tantalus_contracts::FieldNavigationMesh>, ImportError> {
    let source = bundle_path.display().to_string();
    let bundle = UnityRawBundle::open(bundle_path)?;
    let bytes = bundle.read_entry(0, 300 * 1024 * 1024)?;
    let serialized = SerializedFile::parse(&bytes, &source)?;
    let Some(metadata_index) = serialized
        .report()
        .objects
        .iter()
        .position(|object| object.class_id == 142)
    else {
        return Ok(None);
    };
    let metadata = AssetBundleMetadata::parse(
        serialized.object_data(metadata_index)?,
        &format!("{source}:AssetBundle"),
    )?;
    if metadata.atlas_for(field_name).is_none() {
        return Ok(None);
    }
    let navigation_name = format!("{field_name}.bgi");
    let asset = load_text_asset(&serialized, &navigation_name, &source)?;
    let navigation = BgiFile::parse(&asset.script, &navigation_name)?;
    crate::pc::field::bgi_to_navigation_mesh(&navigation).map(Some)
}
