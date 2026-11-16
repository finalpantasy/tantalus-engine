use crate::scene_adapter::{ClientError, ClientSceneAdapter, ResolvedAssets};
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use tantalus_assets::pc::bundled_field::BundledField;
use tantalus_contracts::ContentRevision;

pub fn resolve_field_assets(
    adapter: &mut ClientSceneAdapter,
    field: &BundledField,
) -> Result<ResolvedAssets, ClientError> {
    resolve_field_assets_in(adapter.app_mut().world_mut(), field)
}

/// [`resolve_field_assets`] on a bare `World` (the live window reloads
/// fields from inside a system).
pub fn resolve_field_assets_in(
    world: &mut World,
    field: &BundledField,
) -> Result<ResolvedAssets, ClientError> {
    let rgba = field.atlas.rgba_bytes().map_err(|error| {
        ClientError::new(format!(
            "could not decode atlas {} for `{}`: {error}",
            field.atlas_path_id, field.name
        ))
    })?;
    let image = rgba_image(field.atlas.width, field.atlas.height, rgba)?;
    let handle = world.resource_mut::<Assets<Image>>().add(image);
    Ok(ResolvedAssets {
        content_revision: ContentRevision::default(),
        images: [(field.atlas_id.clone(), handle)].into(),
    })
}

fn rgba_image(width: i32, height: i32, rgba: Vec<u8>) -> Result<Image, ClientError> {
    let width =
        u32::try_from(width).map_err(|_| ClientError::new("field atlas width is not positive"))?;
    let height = u32::try_from(height)
        .map_err(|_| ClientError::new("field atlas height is not positive"))?;
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(ClientError::new(format!(
            "field atlas has {} RGBA bytes for {width}x{height}, expected {expected}",
            rgba.len()
        )));
    }
    Ok(Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::rgba_image;

    #[test]
    fn atlas_pixels_become_a_sized_rgba_image() {
        let image = rgba_image(1, 1, vec![12, 34, 56, 255]).unwrap();
        assert_eq!(image.width(), 1);
        assert_eq!(image.height(), 1);
        assert_eq!(image.data.as_deref(), Some(&[12, 34, 56, 255][..]));
    }

    #[test]
    fn mismatched_atlas_pixels_are_rejected() {
        assert!(rgba_image(2, 2, vec![0; 3]).is_err());
    }
}
