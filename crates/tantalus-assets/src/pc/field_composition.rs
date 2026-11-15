//! CPU composition of a field background (milestones W1 and W2, data side).
//!
//! Everything here is pure except [`compose_bundle`], which walks one
//! `p0data1x` field bundle. Rules come from the pinned Memoria source:
//!
//! * Tile position, `BGSCENE_DEF.CreateScene_OverlayGo` and
//!   `CreateScene_NonCombinedSprite`: scene origin + overlay origin + sprite
//!   offset, one 16x16 native quad per sprite (`BGSCENE_DEF.cs:1840-1878`).
//! * Tile pixels, `BGSCENE_DEF.ExtractSpriteData` (`:715-747`): with the
//!   upscaled atlas, sprite ordinal `n` (running over every overlay in file
//!   order) sits at `padding + (n % columns) * stride`, `columns =
//!   atlas_width / stride`, `stride = tile_size + 2 * padding`.
//! * Initial animation state, `FieldMap.EBG_init` (`:838-848`): every
//!   animation's first frame target overlay gets the Active flag; no other
//!   flag changes before the first animation service.
//! * Animation frames, `FieldMap.BgAnimationService` (`:1538-1597`): a frame
//!   names a target overlay and a tick count; a negative count stops playback.
//!
//! Assumptions that the source does not settle, so they are named here
//! instead of hidden: draw order is "larger depth first" (depth is the Unity
//! z of scene + overlay + sprite, and the Unity camera looks toward larger
//! z), and a transparent sprite follows the PlayStation `abr` blend table
//! because the `abr_*` shaders are Unity assets that are not in the source.

use crate::install::ImportError;
use crate::pc::asset_bundle::AssetBundleMetadata;
use crate::pc::bgs::{BgsFile, BgsOverlay};
use crate::pc::serialized::SerializedFile;
use crate::pc::text_asset::TextAsset;
use crate::pc::texture::Texture2D;
use crate::pc::unity_raw::UnityRawBundle;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Native quad edge in field pixels (`CreateScene_NonCombinedSprite`).
pub const NATIVE_TILE: u32 = 16;
/// `Configuration.Graphics.TileSize` default and the 2 pixel atlas border.
pub const ATLAS_TILE: u32 = 32;
pub const ATLAS_PADDING: u32 = 2;
const OVERLAY_ACTIVE: u8 = 2;
/// Largest canvas edge, in output pixels, before a field is a failure.
const MAX_CANVAS_EDGE: i64 = 16_384;
const MAX_BUNDLE_BYTES: usize = 800 * 1024 * 1024;

/// Decoded atlas pixels, top row first, RGBA8.
pub struct AtlasPixels<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
    pub tile_size: u32,
    pub padding: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayerTile {
    pub sprite_index: u32,
    /// Running sprite number over all overlays; selects the atlas cell.
    pub ordinal: u32,
    /// Native field pixels relative to the scene origin.
    pub x: i32,
    pub y: i32,
    pub depth: i32,
    pub alpha: u8,
    pub transparency: u8,
    /// `[x0, y0, x1, y1]` in atlas pixels, top row first.
    pub atlas_rect: [u32; 4],
    pub atlas_in_bounds: bool,
    /// File size is not the 16x16 the quad is built with.
    pub non_native_size: bool,
    /// Tile lies outside the overlay's declared width and height.
    pub outside_overlay: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OverlayLayer {
    pub overlay_index: u32,
    pub camera: u8,
    pub flags: u8,
    /// Scene current z + overlay current z.
    pub depth: i32,
    /// Drawn in the initial state: Active flag or first frame of an animation.
    pub active: bool,
    /// Position in the camera's back-to-front list.
    pub order: u32,
    pub tiles: Vec<LayerTile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnimationFrame {
    pub target_overlay: u8,
    /// Ticks to hold; negative stops playback on this frame.
    pub ticks: i8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnimationPlan {
    pub index: u32,
    pub camera: u8,
    pub frames: Vec<AnimationFrame>,
    /// Why Memoria's runtime could not consume this record, if so.
    pub unconsumed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CameraPlan {
    pub camera: u8,
    /// Back to front: larger depth first, ties by overlay index.
    pub layers: Vec<OverlayLayer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldLayerPlan {
    pub cameras: Vec<CameraPlan>,
    pub animations: Vec<AnimationPlan>,
}

impl FieldLayerPlan {
    pub fn overlay_count(&self) -> usize {
        self.cameras.iter().map(|camera| camera.layers.len()).sum()
    }
    pub fn tile_count(&self) -> usize {
        self.cameras
            .iter()
            .flat_map(|camera| &camera.layers)
            .map(|layer| layer.tiles.len())
            .sum()
    }
}

/// Builds the camera-ordered layer list for `scene` against an atlas of
/// `atlas_size` pixels laid out as `tile_size` cells with `padding` borders.
pub fn build_layer_plan(
    scene: &BgsFile,
    atlas_size: [u32; 2],
    tile_size: u32,
    padding: u32,
) -> Result<FieldLayerPlan, ImportError> {
    let stride = tile_size + padding * 2;
    let columns = atlas_size[0].checked_div(stride).unwrap_or(0) as usize;
    if columns == 0 {
        return Err(ImportError::invalid(
            "field-composition",
            "atlas holds no tile cells",
        ));
    }
    let mut animations = Vec::with_capacity(scene.animations.len());
    let mut first_frame_targets = BTreeSet::new();
    for (index, animation) in scene.animations.iter().enumerate() {
        let frames: Vec<AnimationFrame> = animation
            .frames
            .iter()
            .map(|&(target, ticks)| AnimationFrame {
                target_overlay: target,
                ticks,
            })
            .collect();
        let unconsumed = animation_problem(scene, animation.camera, &frames);
        if unconsumed.is_none() {
            first_frame_targets.insert(frames[0].target_overlay as usize);
        }
        animations.push(AnimationPlan {
            index: index as u32,
            camera: animation.camera,
            frames,
            unconsumed,
        });
    }

    let mut ordinal = 0_u32;
    let mut by_camera: BTreeMap<u8, Vec<OverlayLayer>> = BTreeMap::new();
    for camera in 0..scene.cameras.len() {
        by_camera.insert(camera as u8, Vec::new());
    }
    for (overlay_index, overlay) in scene.overlays.iter().enumerate() {
        let overlay_depth = scene.current_z as i32 + overlay.current_z as i32;
        let mut tiles = Vec::with_capacity(overlay.sprites.len());
        for (sprite_index, sprite) in overlay.sprites.iter().enumerate() {
            let column = ordinal as usize % columns;
            let row = ordinal as usize / columns;
            let x0 = padding + (column as u32) * stride;
            let y0 = padding + (row as u32) * stride;
            let atlas_rect = [x0, y0, x0 + tile_size, y0 + tile_size];
            let in_bounds = atlas_rect[2] <= atlas_size[0] && atlas_rect[3] <= atlas_size[1];
            let x = scene.current[0] as i32 + overlay.current[0] as i32 + sprite.offset_x as i32;
            let y = scene.current[1] as i32 + overlay.current[1] as i32 + sprite.offset_y as i32;
            tiles.push(LayerTile {
                sprite_index: sprite_index as u32,
                ordinal,
                x,
                y,
                depth: overlay_depth + sprite.depth as i32,
                alpha: sprite.alpha,
                transparency: sprite.transparency,
                atlas_rect,
                atlas_in_bounds: in_bounds,
                non_native_size: sprite.width as u32 != NATIVE_TILE
                    || sprite.height as u32 != NATIVE_TILE,
                outside_overlay: outside_overlay(overlay, sprite.offset_x, sprite.offset_y),
            });
            ordinal += 1;
        }
        by_camera
            .get_mut(&overlay.camera)
            .ok_or_else(|| {
                ImportError::invalid(
                    "field-composition",
                    format!(
                        "overlay {overlay_index} uses missing camera {}",
                        overlay.camera
                    ),
                )
            })?
            .push(OverlayLayer {
                overlay_index: overlay_index as u32,
                camera: overlay.camera,
                flags: overlay.flags,
                depth: overlay_depth,
                active: overlay.flags & OVERLAY_ACTIVE != 0
                    || first_frame_targets.contains(&overlay_index),
                order: 0,
                tiles,
            });
    }
    let cameras = by_camera
        .into_iter()
        .map(|(camera, mut layers)| {
            layers.sort_by_key(|layer| (std::cmp::Reverse(layer.depth), layer.overlay_index));
            for (order, layer) in layers.iter_mut().enumerate() {
                layer.order = order as u32;
            }
            CameraPlan { camera, layers }
        })
        .collect();
    Ok(FieldLayerPlan {
        cameras,
        animations,
    })
}

fn outside_overlay(overlay: &BgsOverlay, offset_x: u16, offset_y: u16) -> bool {
    offset_x as u32 + NATIVE_TILE > overlay.width as u32
        || offset_y as u32 + NATIVE_TILE > overlay.height as u32
}

fn animation_problem(scene: &BgsFile, camera: u8, frames: &[AnimationFrame]) -> Option<String> {
    if frames.is_empty() {
        return Some("no frames (the runtime reads frame 0)".to_owned());
    }
    if camera as usize >= scene.cameras.len() {
        return Some(format!("camera {camera} does not exist"));
    }
    for (index, frame) in frames.iter().enumerate() {
        match scene.overlays.get(frame.target_overlay as usize) {
            None => {
                return Some(format!(
                    "frame {index} targets missing overlay {}",
                    frame.target_overlay
                ));
            }
            Some(target) if target.camera != camera => {
                return Some(format!(
                    "frame {index} targets overlay {} on camera {}, not {camera}",
                    frame.target_overlay, target.camera
                ));
            }
            Some(_) => {}
        }
    }
    None
}

/// One rasterized camera view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    /// Field pixel of the canvas's top-left corner.
    pub origin: [i32; 2],
    pub width: u32,
    pub height: u32,
    /// Output pixels per native field pixel.
    pub scale: u32,
    pub rgba: Vec<u8>,
    pub tiles_drawn: usize,
}

impl Raster {
    pub fn non_empty_pixels(&self) -> usize {
        self.rgba.chunks_exact(4).filter(|px| px[3] != 0).count()
    }
}

/// Rasterizes the camera's layers selected by `include`, back to front.
/// The canvas covers every tile of the camera so frames share one frame of
/// reference. Returns `None` when the camera has no tiles at all.
pub fn rasterize_camera(
    plan: &CameraPlan,
    atlas: &AtlasPixels<'_>,
    include: impl Fn(&OverlayLayer) -> bool,
) -> Result<Option<Raster>, ImportError> {
    if atlas.tile_size == 0 || !atlas.tile_size.is_multiple_of(NATIVE_TILE) {
        return Err(ImportError::invalid(
            "field-composition",
            "atlas tile size must be a multiple of the 16 pixel native tile",
        ));
    }
    if atlas.rgba.len() != atlas.width as usize * atlas.height as usize * 4 {
        return Err(ImportError::invalid(
            "field-composition",
            "atlas pixel buffer does not match its dimensions",
        ));
    }
    let scale = atlas.tile_size / NATIVE_TILE;
    let mut bounds: Option<[i32; 4]> = None;
    for tile in plan.layers.iter().flat_map(|layer| &layer.tiles) {
        let b = bounds.get_or_insert([tile.x, tile.y, tile.x, tile.y]);
        b[0] = b[0].min(tile.x);
        b[1] = b[1].min(tile.y);
        b[2] = b[2].max(tile.x);
        b[3] = b[3].max(tile.y);
    }
    let Some(bounds) = bounds else {
        return Ok(None);
    };
    let native_w = (bounds[2] - bounds[0]) as i64 + NATIVE_TILE as i64;
    let native_h = (bounds[3] - bounds[1]) as i64 + NATIVE_TILE as i64;
    if native_w * scale as i64 > MAX_CANVAS_EDGE || native_h * scale as i64 > MAX_CANVAS_EDGE {
        return Err(ImportError::invalid(
            "field-composition",
            format!("canvas {native_w}x{native_h} native pixels exceeds the budget"),
        ));
    }
    let width = native_w as u32 * scale;
    let height = native_h as u32 * scale;
    let mut canvas = vec![0_u8; width as usize * height as usize * 4];

    let mut tiles: Vec<(&OverlayLayer, &LayerTile)> = plan
        .layers
        .iter()
        .filter(|layer| include(layer))
        .flat_map(|layer| layer.tiles.iter().map(move |tile| (layer, tile)))
        .filter(|(_, tile)| tile.atlas_in_bounds)
        .collect();
    tiles.sort_by_key(|(layer, tile)| {
        (
            std::cmp::Reverse(tile.depth),
            layer.overlay_index,
            tile.sprite_index,
        )
    });
    let side = atlas.tile_size as usize;
    for (_, tile) in &tiles {
        let dest_x = ((tile.x - bounds[0]) as u32 * scale) as usize;
        let dest_y = ((tile.y - bounds[1]) as u32 * scale) as usize;
        for row in 0..side {
            for column in 0..side {
                let source = ((tile.atlas_rect[1] as usize + row) * atlas.width as usize
                    + tile.atlas_rect[0] as usize
                    + column)
                    * 4;
                let src = &atlas.rgba[source..source + 4];
                if src[3] == 0 {
                    continue;
                }
                let target = ((dest_y + row) * width as usize + dest_x + column) * 4;
                blend(&mut canvas[target..target + 4], src, tile);
            }
        }
    }
    Ok(Some(Raster {
        origin: [bounds[0], bounds[1]],
        width,
        height,
        scale,
        rgba: canvas,
        tiles_drawn: tiles.len(),
    }))
}

fn blend(dest: &mut [u8], src: &[u8], tile: &LayerTile) {
    if tile.transparency == 0 || dest[3] == 0 {
        if src[3] == 255 || dest[3] == 0 {
            dest.copy_from_slice(src);
        } else {
            for channel in 0..3 {
                dest[channel] = mix(dest[channel], src[channel], src[3]);
            }
            dest[3] = 255;
        }
        return;
    }
    for channel in 0..3 {
        let back = dest[channel] as i32;
        let front = src[channel] as i32;
        let value = match tile.alpha {
            0 => (back + front) / 2,
            1 => back + front,
            2 => back - front,
            _ => back + front / 4,
        };
        dest[channel] = value.clamp(0, 255) as u8;
    }
}

fn mix(back: u8, front: u8, alpha: u8) -> u8 {
    ((front as u32 * alpha as u32 + back as u32 * (255 - alpha as u32)) / 255) as u8
}

/// How the bytes of a `.bgs` are covered by the records the parser reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ByteAccounting {
    pub total: usize,
    /// Zero-filled runs of under 4 bytes that end on a 4-byte boundary:
    /// alignment padding after a record, read by nothing.
    pub padding: usize,
    pub unaccounted: usize,
    pub overlapping: usize,
    /// Up to eight `[start, length]` runs of unaccounted bytes.
    pub gaps: Vec<[usize; 2]>,
}

const HEADER_BYTES: usize = 52;
const ANIMATION_BYTES: usize = 16;
const OVERLAY_BYTES: usize = 56;
const LIGHT_BYTES: usize = 12;
const CAMERA_BYTES: usize = 52;

/// Marks the header, record tables, frame lists and per-overlay sprite arrays
/// read by [`BgsFile::parse`] and reports the bytes nothing reads or that two
/// records both read. `bytes` must already parse.
pub fn bgs_byte_accounting(bytes: &[u8]) -> ByteAccounting {
    let mut marks = vec![0_u8; bytes.len()];
    let mut mark = |start: usize, length: usize| {
        let end = start.saturating_add(length).min(marks.len());
        for slot in marks.iter_mut().take(end).skip(start.min(end)) {
            *slot = slot.saturating_add(1);
        }
    };
    let u16_at = |at: usize| {
        bytes
            .get(at..at + 2)
            .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
    };
    let u32_at = |at: usize| {
        bytes
            .get(at..at + 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    mark(0, HEADER_BYTES);
    let animations = u16_at(4) as usize;
    let overlays = u16_at(6) as usize;
    let lights = u16_at(8) as usize;
    let cameras = u16_at(10) as usize;
    let animation_offset = u32_at(12) as usize;
    let overlay_offset = u32_at(16) as usize;
    let light_offset = u32_at(20) as usize;
    let camera_offset = u32_at(24) as usize;
    mark(animation_offset, animations * ANIMATION_BYTES);
    mark(overlay_offset, overlays * OVERLAY_BYTES);
    mark(light_offset, lights * LIGHT_BYTES);
    mark(camera_offset, cameras * CAMERA_BYTES);
    for index in 0..animations {
        let record = animation_offset + index * ANIMATION_BYTES;
        let frames = (u32_at(record) >> 8) as usize;
        mark(u32_at(record + 12) as usize, frames * 2);
    }
    for index in 0..overlays {
        let record = overlay_offset + index * OVERLAY_BYTES;
        let sprites = (u32_at(record + 36) >> 16) as usize;
        mark(u32_at(record + 40) as usize, sprites * 4);
        mark(u32_at(record + 44) as usize, sprites * 8);
    }
    let mut accounting = ByteAccounting {
        total: bytes.len(),
        ..ByteAccounting::default()
    };
    let mut run: Option<usize> = None;
    for (position, &count) in marks.iter().chain(std::iter::once(&1)).enumerate() {
        if count == 0 {
            run.get_or_insert(position);
        } else if let Some(start) = run.take() {
            let length = position - start;
            if length < 4 && position % 4 == 0 && bytes[start..position].iter().all(|b| *b == 0) {
                accounting.padding += length;
            } else {
                accounting.unaccounted += length;
                if accounting.gaps.len() < 8 {
                    accounting.gaps.push([start, length]);
                }
            }
        }
        if count > 1 {
            accounting.overlapping += 1;
        }
    }
    accounting
}

#[derive(Debug, Clone, Serialize)]
pub struct CameraReport {
    pub camera: u8,
    pub overlays: usize,
    pub active_layers: usize,
    pub tiles_drawn: usize,
    pub canvas: Option<[u32; 2]>,
    pub non_empty_pixels: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldComposition {
    pub field: String,
    pub bundle: String,
    pub overlay_count: usize,
    /// Overlays drawn in the initial state, over every camera.
    pub layer_count: usize,
    pub tile_count: usize,
    /// Non-empty pixels summed over the camera rasters.
    pub non_empty_pixels: usize,
    pub out_of_bounds_tiles: usize,
    pub tiles_outside_overlay: usize,
    pub non_native_size_tiles: usize,
    pub animation_count: usize,
    pub animation_frame_count: usize,
    pub unconsumed_animations: Vec<String>,
    pub padding_bytes: usize,
    pub unaccounted_bytes: usize,
    pub overlapping_bytes: usize,
    pub unaccounted_gaps: Vec<[usize; 2]>,
    pub cameras: Vec<CameraReport>,
    pub failures: Vec<String>,
    pub passed: bool,
}

pub struct ComposedField {
    pub report: FieldComposition,
    /// Per camera, `(camera, raster)` of the initial state.
    pub rasters: Vec<(u8, Raster)>,
}

/// Composes one parsed field and judges it.
pub fn compose_field(
    field: &str,
    bundle: &str,
    scene: &BgsFile,
    bgs_bytes: &[u8],
    atlas: &AtlasPixels<'_>,
    keep_rasters: bool,
) -> ComposedField {
    let accounting = bgs_byte_accounting(bgs_bytes);
    let mut report = FieldComposition {
        field: field.to_owned(),
        bundle: bundle.to_owned(),
        overlay_count: scene.overlays.len(),
        layer_count: 0,
        tile_count: 0,
        non_empty_pixels: 0,
        out_of_bounds_tiles: 0,
        tiles_outside_overlay: 0,
        non_native_size_tiles: 0,
        animation_count: scene.animations.len(),
        animation_frame_count: scene.animations.iter().map(|a| a.frames.len()).sum(),
        unconsumed_animations: Vec::new(),
        padding_bytes: accounting.padding,
        unaccounted_bytes: accounting.unaccounted,
        overlapping_bytes: accounting.overlapping,
        unaccounted_gaps: accounting.gaps,
        cameras: Vec::new(),
        failures: Vec::new(),
        passed: false,
    };
    let mut rasters = Vec::new();
    match build_layer_plan(
        scene,
        [atlas.width, atlas.height],
        atlas.tile_size,
        atlas.padding,
    ) {
        Err(error) => report.failures.push(error.to_string()),
        Ok(plan) => {
            report.tile_count = plan.tile_count();
            for animation in &plan.animations {
                if let Some(reason) = &animation.unconsumed {
                    report
                        .unconsumed_animations
                        .push(format!("animation {}: {reason}", animation.index));
                }
            }
            for tile in plan
                .cameras
                .iter()
                .flat_map(|c| &c.layers)
                .flat_map(|l| &l.tiles)
            {
                report.out_of_bounds_tiles += usize::from(!tile.atlas_in_bounds);
                report.tiles_outside_overlay += usize::from(tile.outside_overlay);
                report.non_native_size_tiles += usize::from(tile.non_native_size);
            }
            for camera in &plan.cameras {
                let active_layers = camera.layers.iter().filter(|l| l.active).count();
                report.layer_count += active_layers;
                let mut entry = CameraReport {
                    camera: camera.camera,
                    overlays: camera.layers.len(),
                    active_layers,
                    tiles_drawn: 0,
                    canvas: None,
                    non_empty_pixels: 0,
                };
                match rasterize_camera(camera, atlas, |layer| layer.active) {
                    Err(error) => report
                        .failures
                        .push(format!("camera {}: {error}", camera.camera)),
                    Ok(None) => {}
                    Ok(Some(raster)) => {
                        entry.tiles_drawn = raster.tiles_drawn;
                        entry.canvas = Some([raster.width, raster.height]);
                        entry.non_empty_pixels = raster.non_empty_pixels();
                        if active_layers > 0 && entry.non_empty_pixels == 0 {
                            report.failures.push(format!(
                                "camera {} has {active_layers} active layers but no pixels",
                                camera.camera
                            ));
                        }
                        report.non_empty_pixels += entry.non_empty_pixels;
                        if keep_rasters {
                            rasters.push((camera.camera, raster));
                        }
                    }
                }
                report.cameras.push(entry);
            }
            if report
                .cameras
                .first()
                .is_none_or(|c| c.non_empty_pixels == 0)
            {
                report
                    .failures
                    .push("default camera 0 renders no pixels".to_owned());
            }
        }
    }
    if report.out_of_bounds_tiles > 0 {
        report.failures.push(format!(
            "{} tile references fall outside the atlas",
            report.out_of_bounds_tiles
        ));
    }
    if !report.unconsumed_animations.is_empty() {
        report.failures.push(format!(
            "{} animation records cannot be consumed",
            report.unconsumed_animations.len()
        ));
    }
    if report.unaccounted_bytes > 0 {
        report.failures.push(format!(
            "{} bytes of the scene are read by no record",
            report.unaccounted_bytes
        ));
    }
    report.passed = report.failures.is_empty();
    ComposedField { report, rasters }
}

/// Language suffixes of `{field}_{SYMBOL}.bgs` title replacements
/// (`BGSCENE_DEF._LoadDummyEBG`): they swap overlays of the base field and
/// reuse its atlas, so they are not fields of their own.
pub const LOCALE_SYMBOLS: [&str; 6] = ["ES", "FR", "GR", "IT", "JP", "UK"];

#[derive(Debug, Clone, Serialize)]
pub struct LocaleVariant {
    pub name: String,
    pub base_field: String,
    pub bundle: String,
    pub overlay_count: usize,
    pub base_overlay_count: usize,
    pub padding_bytes: usize,
    pub unaccounted_bytes: usize,
    pub problems: Vec<String>,
}

#[derive(Debug, Default)]
pub struct BundleOutcome {
    pub fields: usize,
    pub locale_variants: Vec<LocaleVariant>,
}

fn locale_base(name: &str, all: &BTreeMap<String, Vec<u8>>) -> Option<String> {
    let (base, symbol) = name.rsplit_once('_')?;
    (LOCALE_SYMBOLS.contains(&symbol) && all.contains_key(base)).then(|| base.to_owned())
}

/// Composes every field of one field bundle. `on_field` receives each result
/// as soon as it exists so a caller can write PNGs without holding them all.
pub fn compose_bundle(
    path: &Path,
    keep_rasters: bool,
    mut on_field: impl FnMut(ComposedField),
) -> Result<BundleOutcome, ImportError> {
    let source = path.display().to_string();
    let bundle_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.clone());
    let bundle = UnityRawBundle::open(path)?;
    let bytes = bundle.read_entry(0, MAX_BUNDLE_BYTES)?;
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
    let mut scenes: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (index, object) in serialized.report().objects.iter().enumerate() {
        if object.class_id != 49 {
            continue;
        }
        let asset = TextAsset::parse(serialized.object_data(index)?, &source)?;
        if let Some(name) = asset.name.strip_suffix(".bgs") {
            scenes.insert(name.to_owned(), asset.script);
        }
    }
    let mut outcome = BundleOutcome::default();
    let locale: Vec<(String, String)> = scenes
        .keys()
        .filter_map(|name| locale_base(name, &scenes).map(|base| (name.clone(), base)))
        .collect();
    for (name, base) in &locale {
        let script = &scenes[name];
        let mut problems = Vec::new();
        let mut overlay_count = 0;
        let mut base_overlay_count = 0;
        match (
            BgsFile::parse(script, name),
            BgsFile::parse(&scenes[base], base),
        ) {
            (Ok(variant), Ok(main)) => {
                overlay_count = variant.overlays.len();
                base_overlay_count = main.overlays.len();
                if overlay_count != base_overlay_count {
                    problems.push("overlay count differs from the base field".to_owned());
                }
            }
            (Err(error), _) => problems.push(format!("does not parse: {error}")),
            (_, Err(error)) => problems.push(format!("base field does not parse: {error}")),
        }
        let accounting = bgs_byte_accounting(script);
        outcome.locale_variants.push(LocaleVariant {
            name: name.clone(),
            base_field: base.clone(),
            bundle: bundle_name.clone(),
            overlay_count,
            base_overlay_count,
            padding_bytes: accounting.padding,
            unaccounted_bytes: accounting.unaccounted,
            problems,
        });
    }
    let skipped: BTreeSet<&String> = locale.iter().map(|(name, _)| name).collect();
    outcome.fields = scenes.len() - skipped.len();
    for (field, script) in scenes.iter().filter(|(name, _)| !skipped.contains(name)) {
        let (field, script) = (field.clone(), script.clone());
        let composed = compose_one(
            &serialized,
            &metadata,
            &bundle_name,
            &field,
            &script,
            keep_rasters,
        );
        on_field(composed);
    }
    Ok(outcome)
}

fn compose_one(
    serialized: &SerializedFile<'_>,
    metadata: &AssetBundleMetadata,
    bundle_name: &str,
    field: &str,
    script: &[u8],
    keep_rasters: bool,
) -> ComposedField {
    let failed = |reason: String| ComposedField {
        report: FieldComposition {
            field: field.to_owned(),
            bundle: bundle_name.to_owned(),
            overlay_count: 0,
            layer_count: 0,
            tile_count: 0,
            non_empty_pixels: 0,
            out_of_bounds_tiles: 0,
            tiles_outside_overlay: 0,
            non_native_size_tiles: 0,
            animation_count: 0,
            animation_frame_count: 0,
            unconsumed_animations: Vec::new(),
            padding_bytes: 0,
            unaccounted_bytes: 0,
            overlapping_bytes: 0,
            unaccounted_gaps: Vec::new(),
            cameras: Vec::new(),
            failures: vec![reason],
            passed: false,
        },
        rasters: Vec::new(),
    };
    let scene = match BgsFile::parse(script, field) {
        Ok(scene) => scene,
        Err(error) => return failed(format!("scene does not parse: {error}")),
    };
    let Some(info) = metadata.atlas_for(field) else {
        return failed("no atlas is registered for the field".to_owned());
    };
    let Some(atlas_index) = serialized
        .report()
        .objects
        .iter()
        .position(|object| object.class_id == 28 && object.path_id == info.path_id)
    else {
        return failed(format!(
            "atlas path ID {} is not a Texture2D in this bundle",
            info.path_id
        ));
    };
    let texture = match serialized
        .object_data(atlas_index)
        .and_then(|data| Texture2D::parse(data, field))
    {
        Ok(texture) => texture,
        Err(error) => return failed(format!("atlas does not parse: {error}")),
    };
    let rgba = match texture.rgba_bytes() {
        Ok(rgba) => rgba,
        Err(error) => return failed(format!("atlas does not decode: {error}")),
    };
    let (Ok(width), Ok(height)) = (u32::try_from(texture.width), u32::try_from(texture.height))
    else {
        return failed("atlas has a negative size".to_owned());
    };
    compose_field(
        field,
        bundle_name,
        &scene,
        script,
        &AtlasPixels {
            width,
            height,
            rgba: &rgba,
            tile_size: ATLAS_TILE,
            padding: ATLAS_PADDING,
        },
        keep_rasters,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pc::bgs::{BgsAnimation, BgsCamera, BgsSprite};

    fn camera() -> BgsCamera {
        BgsCamera {
            projection: 100,
            rotation: [[0; 3]; 3],
            translation: [0; 3],
            center_offset: [0; 2],
            width: 320,
            height: 224,
            viewport_x: [0; 2],
            viewport_y: [0; 2],
            depth_offset: 0,
        }
    }

    fn sprite(offset_x: u16, offset_y: u16, depth: u16) -> BgsSprite {
        BgsSprite {
            clut_y: 0,
            clut_x: 0,
            texture_y: 0,
            texture_x: 0,
            resolution: 0,
            alpha: 0,
            v: 0,
            u: 0,
            height: 16,
            width: 16,
            transparency: 0,
            depth,
            offset_y,
            offset_x,
        }
    }

    fn overlay(camera: u8, flags: u8, z: u16, sprites: Vec<BgsSprite>) -> BgsOverlay {
        BgsOverlay {
            flags,
            current_z: z,
            original_z: z,
            width: 64,
            height: 64,
            origin: [0; 2],
            current: [0; 2],
            limits: [0; 4],
            screen: [0; 2],
            scroll: [0; 2],
            fraction: [0; 2],
            camera,
            is_x_offset: false,
            viewport: 0,
            sprites,
        }
    }

    fn scene(overlays: Vec<BgsOverlay>, animations: Vec<BgsAnimation>) -> BgsFile {
        BgsFile {
            scene_length: 0,
            depth_bit_shift: 1,
            origin_z: 0,
            current_z: 10,
            origin: [0; 2],
            current: [0; 2],
            limits: [0; 4],
            screen: [0; 2],
            animations,
            overlays,
            lights: Vec::new(),
            cameras: vec![camera(), camera()],
        }
    }

    fn animation(camera: u8, frames: Vec<(u8, i8)>) -> BgsAnimation {
        BgsAnimation {
            flags: 1,
            camera,
            current_frame: 0,
            frame_rate: 256,
            counter: 0,
            frames,
        }
    }

    /// 76x76 atlas: 2x2 cells of 32 pixels with 2 pixel borders.
    /// Cell 0 solid red, 1 solid green, 2 solid blue, 3 half-covered blue.
    fn atlas_pixels() -> Vec<u8> {
        let mut rgba = vec![0_u8; 76 * 76 * 4];
        let colors = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [0, 0, 255]];
        for (cell, color) in colors.iter().enumerate() {
            let x0 = 2 + (cell % 2) * 36;
            let y0 = 2 + (cell / 2) * 36;
            for row in 0..32 {
                for column in 0..32 {
                    if cell == 3 && column >= 16 {
                        continue;
                    }
                    let at = ((y0 + row) * 76 + x0 + column) * 4;
                    rgba[at..at + 3].copy_from_slice(color);
                    rgba[at + 3] = 255;
                }
            }
        }
        rgba
    }

    fn atlas(rgba: &[u8]) -> AtlasPixels<'_> {
        AtlasPixels {
            width: 76,
            height: 76,
            rgba,
            tile_size: 32,
            padding: 2,
        }
    }

    #[test]
    fn layers_sort_back_to_front_per_camera_and_keep_file_ordinals() {
        let near = overlay(0, 2, 1, vec![sprite(0, 0, 0)]);
        let far = overlay(0, 2, 9, vec![sprite(0, 0, 0)]);
        let other = overlay(1, 2, 5, vec![sprite(0, 0, 0)]);
        let plan =
            build_layer_plan(&scene(vec![near, far, other], vec![]), [76, 76], 32, 2).unwrap();
        let first: Vec<u32> = plan.cameras[0]
            .layers
            .iter()
            .map(|l| l.overlay_index)
            .collect();
        assert_eq!(first, [1, 0]);
        assert_eq!(plan.cameras[0].layers[0].depth, 19);
        assert_eq!(plan.cameras[1].layers[0].overlay_index, 2);
        assert_eq!(plan.cameras[1].layers[0].tiles[0].ordinal, 2);
        assert_eq!(
            plan.cameras[1].layers[0].tiles[0].atlas_rect,
            [2, 38, 34, 70]
        );
    }

    #[test]
    fn nearer_tile_covers_farther_tile_and_fills_the_canvas() {
        let back = overlay(0, 2, 9, vec![sprite(0, 0, 0)]);
        let front = overlay(0, 2, 1, vec![sprite(16, 0, 0)]);
        let plan = build_layer_plan(&scene(vec![back, front], vec![]), [76, 76], 32, 2).unwrap();
        let rgba = atlas_pixels();
        let raster = rasterize_camera(&plan.cameras[0], &atlas(&rgba), |l| l.active)
            .unwrap()
            .unwrap();
        assert_eq!((raster.width, raster.height, raster.scale), (64, 32, 2));
        assert_eq!(raster.non_empty_pixels(), 64 * 32);
        assert_eq!(&raster.rgba[..4], &[255, 0, 0, 255]);
        let right = (63) * 4;
        assert_eq!(&raster.rgba[right..right + 4], &[0, 255, 0, 255]);
    }

    #[test]
    fn overlapping_tiles_resolve_by_depth_not_file_order() {
        let a = overlay(0, 2, 1, vec![sprite(0, 0, 0)]);
        let b = overlay(0, 2, 9, vec![sprite(0, 0, 0)]);
        let plan = build_layer_plan(&scene(vec![a, b], vec![]), [76, 76], 32, 2).unwrap();
        let rgba = atlas_pixels();
        let raster = rasterize_camera(&plan.cameras[0], &atlas(&rgba), |l| l.active)
            .unwrap()
            .unwrap();
        // Overlay 0 (depth 11) is nearer than overlay 1 (depth 19): red on top.
        assert_eq!(&raster.rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn inactive_overlays_draw_only_when_an_animation_starts_on_them() {
        let hidden = overlay(0, 0, 3, vec![sprite(0, 0, 0)]);
        let animated = overlay(0, 0, 2, vec![sprite(0, 0, 0)]);
        let second = overlay(0, 0, 1, vec![sprite(0, 0, 0)]);
        let plan = build_layer_plan(
            &scene(
                vec![hidden, animated, second],
                vec![animation(0, vec![(1, 4), (2, -1)])],
            ),
            [76, 76],
            32,
            2,
        )
        .unwrap();
        let active: Vec<bool> = plan.cameras[0].layers.iter().map(|l| l.active).collect();
        // Back to front: overlay 0 (hidden), 1 (frame 0), 2 (frame 1, not yet).
        assert_eq!(active, [false, true, false]);
        assert_eq!(plan.animations[0].frames[1].ticks, -1);
        assert!(plan.animations[0].unconsumed.is_none());
    }

    #[test]
    fn broken_animation_records_are_named_not_dropped() {
        let layers = vec![
            overlay(0, 2, 1, vec![sprite(0, 0, 0)]),
            overlay(1, 2, 1, vec![sprite(0, 0, 0)]),
        ];
        let animations = vec![
            animation(0, vec![]),
            animation(0, vec![(7, 1)]),
            animation(0, vec![(1, 1)]),
            animation(5, vec![(0, 1)]),
            animation(0, vec![(0, 1)]),
        ];
        let plan = build_layer_plan(&scene(layers, animations), [76, 76], 32, 2).unwrap();
        let problems: Vec<bool> = plan
            .animations
            .iter()
            .map(|a| a.unconsumed.is_some())
            .collect();
        assert_eq!(problems, [true, true, true, true, false]);
        assert!(
            plan.animations[1]
                .unconsumed
                .as_ref()
                .unwrap()
                .contains("missing overlay 7")
        );
    }

    #[test]
    fn out_of_bounds_tiles_fail_the_field_and_are_counted() {
        // A 2x2 cell atlas holds four tiles; the fifth is outside.
        let sprites = (0..5).map(|n| sprite(n * 16, 0, 0)).collect();
        let file = scene(vec![overlay(0, 2, 1, sprites)], vec![]);
        let rgba = atlas_pixels();
        let composed = compose_field("F", "b", &file, &[], &atlas(&rgba), false);
        assert_eq!(composed.report.out_of_bounds_tiles, 1);
        assert!(!composed.report.passed);
        assert!(
            composed
                .report
                .failures
                .iter()
                .any(|f| f.contains("outside the atlas"))
        );
    }

    #[test]
    fn transparent_pixels_and_additive_blend_follow_the_table() {
        let mut add = sprite(0, 0, 0);
        add.transparency = 1;
        add.alpha = 1;
        let back = overlay(0, 2, 9, vec![sprite(0, 0, 0)]);
        let front = overlay(0, 2, 1, vec![add]);
        let mut file = scene(vec![back, front], vec![]);
        // Sprite 1 would use cell 1 (green); put the red cell under it.
        file.overlays[1].sprites[0].offset_x = 0;
        let plan = build_layer_plan(&file, [76, 76], 32, 2).unwrap();
        let rgba = atlas_pixels();
        let raster = rasterize_camera(&plan.cameras[0], &atlas(&rgba), |l| l.active)
            .unwrap()
            .unwrap();
        assert_eq!(&raster.rgba[..4], &[255, 255, 0, 255]);
    }

    #[test]
    fn cells_with_transparent_pixels_leave_the_canvas_empty_there() {
        let file = scene(vec![overlay(0, 2, 1, vec![sprite(0, 0, 0); 4])], vec![]);
        let mut file = file;
        for (n, s) in file.overlays[0].sprites.iter_mut().enumerate() {
            s.offset_x = (n as u16) * 16;
        }
        let plan = build_layer_plan(&file, [76, 76], 32, 2).unwrap();
        let rgba = atlas_pixels();
        let raster = rasterize_camera(&plan.cameras[0], &atlas(&rgba), |l| l.active)
            .unwrap()
            .unwrap();
        // Three full tiles plus half of the fourth, 32x32 output pixels each.
        assert_eq!(raster.non_empty_pixels(), 32 * 32 * 3 + 16 * 32);
    }

    #[test]
    fn overlays_without_tiles_have_no_raster() {
        let plan = build_layer_plan(
            &scene(vec![overlay(0, 2, 1, vec![])], vec![]),
            [76, 76],
            32,
            2,
        )
        .unwrap();
        let rgba = atlas_pixels();
        assert!(
            rasterize_camera(&plan.cameras[0], &atlas(&rgba), |_| true)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn atlas_without_cells_is_an_error() {
        assert!(build_layer_plan(&scene(vec![], vec![]), [10, 10], 32, 2).is_err());
    }

    fn u16le(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn i16le(bytes: &mut Vec<u8>, value: i16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn u32le(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Same layout as `tests/bgs.rs`: one record of every kind, 202 bytes.
    fn encoded_scene() -> Vec<u8> {
        let mut bytes = Vec::new();
        u16le(&mut bytes, 202);
        u16le(&mut bytes, 1);
        for _ in 0..4 {
            u16le(&mut bytes, 1);
        }
        for offset in [52, 68, 124, 136] {
            u32le(&mut bytes, offset);
        }
        for _ in 0..12 {
            i16le(&mut bytes, 0);
        }
        u32le(&mut bytes, (1 << 8) | 2);
        u32le(&mut bytes, 0);
        i16le(&mut bytes, 256);
        u16le(&mut bytes, 0);
        u32le(&mut bytes, 200);
        u32le(&mut bytes, 2 | (3 << 8) | (4 << 20));
        u16le(&mut bytes, 16);
        u16le(&mut bytes, 16);
        for _ in 0..14 {
            i16le(&mut bytes, 0);
        }
        u32le(&mut bytes, 1 << 16);
        u32le(&mut bytes, 196);
        u32le(&mut bytes, 188);
        u32le(&mut bytes, 0);
        u32le(&mut bytes, 0);
        for value in [1, 2, 3] {
            i16le(&mut bytes, value);
        }
        u16le(&mut bytes, 0);
        bytes.extend_from_slice(&[4, 5, 6, 0]);
        u16le(&mut bytes, 100);
        for row in [[4096, 0, 0], [0, 4096, 0], [0, 0, 4096]] {
            for value in row {
                i16le(&mut bytes, value);
            }
        }
        for _ in 0..3 {
            u32le(&mut bytes, 0);
        }
        for _ in 0..2 {
            i16le(&mut bytes, 0);
        }
        i16le(&mut bytes, 320);
        i16le(&mut bytes, 224);
        for _ in 0..4 {
            i16le(&mut bytes, 0);
        }
        u32le(&mut bytes, 0);
        u32le(&mut bytes, 0);
        u32le(&mut bytes, (16 << 8) | (16 << 18));
        u32le(&mut bytes, 7 | (8 << 12) | (9 << 22));
        bytes.extend_from_slice(&[1, 0xfe]);
        bytes
    }

    #[test]
    fn a_fully_read_scene_has_no_unaccounted_bytes() {
        let bytes = encoded_scene();
        BgsFile::parse(&bytes, "synthetic").unwrap();
        let accounting = bgs_byte_accounting(&bytes);
        assert_eq!(accounting.total, 202);
        assert_eq!(accounting.unaccounted, 0);
        assert_eq!(accounting.overlapping, 0);
    }

    #[test]
    fn zero_alignment_padding_is_accounted_for_separately() {
        let mut bytes = encoded_scene();
        bytes.extend_from_slice(&[0, 0]);
        bytes[0..2].copy_from_slice(&204_u16.to_le_bytes());
        let accounting = bgs_byte_accounting(&bytes);
        assert_eq!((accounting.padding, accounting.unaccounted), (2, 0));
    }

    #[test]
    fn trailing_bytes_nothing_reads_are_reported_with_their_position() {
        let mut bytes = encoded_scene();
        // Frame list at 200 grows two bytes of slack and the length follows.
        bytes.extend_from_slice(&[9, 9, 9]);
        bytes[0..2].copy_from_slice(&205_u16.to_le_bytes());
        BgsFile::parse(&bytes, "slack").unwrap();
        let accounting = bgs_byte_accounting(&bytes);
        assert_eq!(accounting.unaccounted, 3);
        assert_eq!(accounting.gaps, vec![[202, 3]]);
    }

    #[test]
    fn a_shared_sprite_array_counts_as_overlap() {
        let mut bytes = encoded_scene();
        // Point the frame list at the sprite location array.
        bytes[52 + 12..52 + 16].copy_from_slice(&196_u32.to_le_bytes());
        let accounting = bgs_byte_accounting(&bytes);
        assert_eq!(accounting.overlapping, 2);
        assert_eq!(accounting.unaccounted, 2);
    }
}
