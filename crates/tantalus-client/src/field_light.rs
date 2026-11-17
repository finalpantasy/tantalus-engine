//! Field character light and shadows from the field's map config
//! (`CommonAsset/MapConfigData/<field>`, `p0data7.bin`).
//!
//! Sources: Memoria `fldmcf.FieldMCFService` (which char and floor light
//! apply), `fldchar` (the light becomes the actor material `_Color`),
//! `ff9shadow.FF9ShadowSetAmpField`/`FF9ShadowSetScaleField` and
//! `FieldMapActor.CreateShadowMesh` (a `shadow_plate` quad on the floor),
//! and the `PSX/FieldMapActorShadow` shader (reverse-subtract blend).
//! Script light changes (`CHRCOLOR`, the `MCF_*` codes, `SHADOW*`) come in
//! through `ActorShadowScript` and `apply_light_changes`.

use std::collections::HashMap;
use std::path::Path;
use tantalus_assets::pc::asset_bundle::AssetBundleMetadata;
use tantalus_assets::pc::map_config::{MapChar, MapConfig, MapLight};
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::text_asset::TextAsset;
use tantalus_assets::pc::texture::Texture2D;
use tantalus_assets::pc::unity_raw::UnityRawBundle;
use tantalus_contracts::FieldNavigationMesh;
use tantalus_game::field::world::{
    ActorShadowScript, CHAR_ATTR_LADDER, CHAR_ATTR_SHADOW_OFF, LightChange,
};

/// `fldmcf.LIGHT_TYPE_FLOOR`, `_DEFAULT`, `_LADDER`.
const LIGHT_FLOOR: i8 = 0;
const LIGHT_DEFAULT: i8 = 1;
const LIGHT_LADDER: i8 = 2;
/// `FieldMapActor.shadowHeightOffset`.
pub const SHADOW_HEIGHT_OFFSET: f32 = 5.0;
/// `CreateShadowMesh` vertex color alpha.
pub const SHADOW_VERTEX_ALPHA: f32 = 0.6;

/// How one actor is lit this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActorLight {
    /// The material `_Color` as `PSX/FieldMapActor` applies it: twice the
    /// color byte over 255, so the default 128 is 1.
    pub tint: [f32; 3],
    pub shadow: Option<ShadowLook>,
}

impl Default for ActorLight {
    fn default() -> Self {
        Self {
            tint: [1.0; 3],
            shadow: None,
        }
    }
}

/// A shadow quad's color and half extents in field units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowLook {
    /// `_Color` rgb: `(byte)(amp * 2) / 255`.
    pub strength: f32,
    /// `FF9ShadowSetScaleField`: `224 * r / 16` and `192 * r / 16` (the
    /// quad spans -1..1).
    pub half: [f32; 2],
    /// `SHADOWOFFSET`: x and z from the actor's foot.
    pub offset: [f32; 2],
}

/// `FieldMCFService` for one character: `model` is its model id, `floor`
/// its walkmesh floor. Without a config the material keeps its 0.5
/// `_Color` and the shadow its black one, so the actor is untinted and
/// casts no shadow.
pub fn actor_light(config: Option<&MapConfig>, model: u16, floor: Option<i16>) -> ActorLight {
    actor_light_scripted(config, model, floor, &ActorShadowScript::default())
}

/// [`actor_light`] with what the field scripts did to the actor: its own
/// `CHRCOLOR` color (which the map light no longer overrides), the ladder
/// light while it climbs (`FF9Char.attr` 4), and the `SHADOW*` opcodes'
/// strength, scale, offset and visibility. The scripts' values win over the
/// config's; the reference applies the config's again only when the actor
/// changes floor, which this does not model.
pub fn actor_light_scripted(
    config: Option<&MapConfig>,
    model: u16,
    floor: Option<i16>,
    script: &ActorShadowScript,
) -> ActorLight {
    let mut light = match config {
        Some(config) => config_light(config, model, floor, script),
        None => ActorLight::default(),
    };
    if let Some(color) = script.color {
        light.tint = color.map(|byte| 2.0 * f32::from(byte) / 255.0);
    }
    let casts = script.attr & CHAR_ATTR_SHADOW_OFF == 0;
    let base = light.shadow;
    let strength = script
        .amp
        // `FF9ShadowSetAmpField` then `(Byte)(amp * 2)`.
        .map(|amp| f32::from(((amp as u8 as i32) * 2) as u8) / 255.0)
        .or(base.map(|shadow| shadow.strength));
    let half = script
        .scale
        .map(|[x, z]| [224.0 * x as f32 / 16.0, 192.0 * z as f32 / 16.0])
        .or(base.map(|shadow| shadow.half));
    light.shadow = match (casts, strength, half) {
        (true, Some(strength), Some(half)) if strength > 0.0 && half[0] > 0.0 => Some(ShadowLook {
            strength,
            half,
            offset: script
                .offset
                .map_or([0.0; 2], |[x, z]| [x as f32, z as f32]),
        }),
        _ => None,
    };
    light
}

fn config_light(
    config: &MapConfig,
    model: u16,
    floor: Option<i16>,
    script: &ActorShadowScript,
) -> ActorLight {
    let char_light = char_by_model(config, model);
    let default_char = char_by_model(config, 0xFFFF);
    let (default_light, ladder_light) = light_defaults(config);
    let floor_light = floor.and_then(|floor| light_by_floor(config, floor));
    let char_color = char_light
        .or(default_char)
        .map_or([128, 128, 128], |char| char.color);
    let on_ladder = script.attr & CHAR_ATTR_LADDER != 0;
    let light_color = if on_ladder && ladder_light.is_some() {
        ladder_light
    } else {
        floor_light.or(default_light)
    }
    .map_or([0, 0, 0], |light| {
        [light.color[0], light.color[1], light.color[2]]
    });
    let tint = [0, 1, 2].map(|channel| {
        // `(Byte)(charColor + lightColor)`: wraps like the C# cast.
        let byte = (i32::from(char_color[channel]) + i32::from(light_color[channel])) as u8;
        2.0 * f32::from(byte) / 255.0
    });
    let shadow = char_light.or(default_char).map(|char| {
        let light = floor_light.or(default_light);
        let intensity = i32::from(char.shadow_intensity)
            + light.map_or(0, |light| i32::from(light.shadow_intensity));
        let radius =
            i32::from(char.shadow_radius) + light.map_or(0, |light| i32::from(light.shadow_radius));
        ShadowLook {
            strength: f32::from((intensity * 2) as u8) / 255.0,
            half: [224.0 * radius as f32 / 16.0, 192.0 * radius as f32 / 16.0],
            offset: [0.0; 2],
        }
    });
    ActorLight { tint, shadow }
}

/// `fldmcf.SetMCFDefaultLight`, `SetMCFFloorLight` and `SetMCFModelLight`
/// replayed on `config` (made on the first call, as `EnsureLightConfig`
/// does).
pub fn apply_light_changes(config: &mut Option<MapConfig>, changes: &[LightChange]) {
    for change in changes {
        let config = config.get_or_insert_with(|| MapConfig {
            attr: 0,
            version: 3,
            bg_no: 0,
            light_count: 0,
            light_use: 0,
            char_count: 0,
            char_use: 0,
            evt_count: 0,
            evt_use: 0,
            lights: Vec::new(),
            chars: Vec::new(),
            trailing: Vec::new(),
        });
        match *change {
            LightChange::Default {
                kind: -1,
                color,
                shadow_intensity,
                shadow_scale,
            } => set_model_light(config, -1, color, shadow_intensity, shadow_scale),
            LightChange::Default {
                kind,
                color,
                shadow_intensity,
                shadow_scale,
            } => set_light(config, kind, -1, color, shadow_intensity, shadow_scale),
            LightChange::Floor {
                floor,
                color,
                shadow_intensity,
                shadow_scale,
            } => set_light(
                config,
                LIGHT_FLOOR,
                floor,
                color,
                shadow_intensity,
                shadow_scale,
            ),
            LightChange::Model {
                model,
                color,
                shadow_intensity,
                shadow_scale,
            } => set_model_light(config, model, color, shadow_intensity, shadow_scale),
        }
    }
}

/// `GetOrCreateLight`: an existing light of the kind that names `floor`
/// (compared as `(byte)floor == floorIndex`, so a negative index never
/// matches), else a new one.
fn set_light(
    config: &mut MapConfig,
    kind: i8,
    floor: i8,
    color: [u8; 3],
    shadow_intensity: i8,
    shadow_scale: i8,
) {
    let at = config
        .lights
        .iter()
        .position(|light| {
            light.kind == kind
                && light
                    .floor
                    .iter()
                    .any(|entry| i32::from(*entry as u8) == i32::from(floor))
        })
        .unwrap_or_else(|| {
            config.lights.push(MapLight {
                kind,
                no: 0,
                shadow_intensity: 0,
                shadow_radius: 0,
                color: [0; 4],
                floor: [floor, -1, -1, -1],
            });
            config.light_count = config.light_count.wrapping_add(1);
            config.lights.len() - 1
        });
    let light = &mut config.lights[at];
    light.color = [
        i16::from(color[0] as i8),
        i16::from(color[1] as i8),
        i16::from(color[2] as i8),
        light.color[3],
    ];
    light.shadow_intensity = i16::from(shadow_intensity);
    light.shadow_radius = shadow_scale;
    config.light_use = 1;
}

/// `GetOrCreateCharLight`; a model of -1 is every model (`0xFFFF`).
fn set_model_light(
    config: &mut MapConfig,
    model: i32,
    color: [u8; 3],
    shadow_intensity: i8,
    shadow_scale: i8,
) {
    let geo_no = (model & 0xFFFF) as u16;
    let at = config
        .chars
        .iter()
        .position(|char| char.geo_no == geo_no)
        .unwrap_or_else(|| {
            config.chars.push(MapChar {
                geo_no,
                shadow_intensity: 0,
                shadow_radius: 0,
                color: [0; 3],
                shadow_z: 0,
            });
            config.char_count = config.char_count.wrapping_add(1);
            config.chars.len() - 1
        });
    let char = &mut config.chars[at];
    char.color = [
        i16::from(color[0] as i8),
        i16::from(color[1] as i8),
        i16::from(color[2] as i8),
    ];
    char.shadow_intensity = i16::from(shadow_intensity);
    char.shadow_radius = shadow_scale;
    config.char_use = 1;
}

fn char_by_model(config: &MapConfig, model: u16) -> Option<&MapChar> {
    if config.char_use == 0 {
        return None;
    }
    config.chars.iter().find(|char| char.geo_no == model)
}

/// The last default and ladder lights, as the reference's loop keeps them.
fn light_defaults(config: &MapConfig) -> (Option<&MapLight>, Option<&MapLight>) {
    if config.light_use == 0 {
        return (None, None);
    }
    let mut found = (None, None);
    for light in &config.lights {
        match light.kind {
            LIGHT_DEFAULT => found.0 = Some(light),
            LIGHT_LADDER => found.1 = Some(light),
            _ => {}
        }
    }
    found
}

fn light_by_floor(config: &MapConfig, floor: i16) -> Option<&MapLight> {
    if config.light_use == 0 || floor & 0xFF == 0xFF {
        return None;
    }
    config.lights.iter().find(|light| {
        light.kind == LIGHT_FLOOR
            && light
                .floor
                .iter()
                .any(|entry| i16::from(*entry as u8) == floor)
    })
}

/// The floor of the walkmesh triangle under `position`: the triangle that
/// contains it on x/z with the nearest height, over every floor. A height
/// the scripts left unset (|y| at or past 32768) takes the active floor.
pub fn actor_floor(mesh: &FieldNavigationMesh, position: [f32; 3]) -> Option<i16> {
    let navigation = tantalus_game::navigation::FieldNavigation::new(mesh).ok()?;
    let point = tantalus_contracts::Point3 {
        x: position[0].round() as i32,
        y: if position[1].abs() >= 32768.0 {
            0
        } else {
            position[1].round() as i32
        },
        z: position[2].round() as i32,
    };
    if position[1].abs() >= 32768.0 {
        let projection = navigation.project(point).ok()?;
        return mesh
            .triangles
            .get(projection.triangle_index)
            .map(|triangle| triangle.floor);
    }
    let mut floors: Vec<i16> = mesh
        .triangles
        .iter()
        .map(|triangle| triangle.floor)
        .collect();
    floors.sort_unstable();
    floors.dedup();
    floors
        .into_iter()
        .filter_map(|floor| {
            let projection = navigation.project_on_floor(point, floor).ok()?;
            let at = projection.position;
            let dx = i64::from(at.x) - i64::from(point.x);
            let dz = i64::from(at.z) - i64::from(point.z);
            let dy = (i64::from(at.y) - i64::from(point.y)).abs();
            Some(((dx * dx + dz * dz, dy), floor))
        })
        .min()
        .map(|(_, floor)| floor)
}

/// A shadow texture: width, height and top-down RGBA.
pub type ShadowPlate = (usize, usize, Vec<u8>);

/// What `p0data7.bin` holds for field light: every field's map config,
/// keyed by the lowercase field name (`EVT_...`), and
/// `CommonAsset/Common/shadow_plate`. `AssetManager.Load` takes the bundle
/// copy before the one in `resources.assets` (whose color is black, so the
/// shadow shader would discard all of it).
pub fn load_field_light(
    p0data7: &Path,
) -> Result<(HashMap<String, MapConfig>, Option<ShadowPlate>), String> {
    let source = p0data7.display().to_string();
    let bundle = UnityRawBundle::open(p0data7).map_err(|error| error.to_string())?;
    let bytes = bundle
        .read_entry(0, 600 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    let serialized = SerializedFile::parse(&bytes, &source).map_err(|error| error.to_string())?;
    let report = serialized.report();
    let container = report
        .objects
        .iter()
        .position(|object| object.class_id == 142)
        .ok_or("p0data7.bin has no AssetBundle metadata")?;
    let metadata = AssetBundleMetadata::parse(
        serialized
            .object_data(container)
            .map_err(|error| error.to_string())?,
        &source,
    )
    .map_err(|error| error.to_string())?;
    let object = |path_id: i64| {
        let index = report
            .objects
            .iter()
            .position(|object| object.path_id == path_id)?;
        serialized.object_data(index).ok()
    };
    let mut configs = HashMap::new();
    let mut plate = None;
    for (asset_path, info) in &metadata.container {
        let lower = asset_path.to_ascii_lowercase();
        if lower.contains("commonasset/common/shadow_plate") {
            plate = object(info.path_id)
                .and_then(|data| Texture2D::parse(data, asset_path).ok())
                .and_then(|texture| {
                    let rgba = texture.rgba_bytes().ok()?;
                    Some((texture.width as usize, texture.height as usize, rgba))
                });
            continue;
        }
        let Some(file) = lower.split("/mapconfigdata/").nth(1) else {
            continue;
        };
        let name = file.split('.').next().unwrap_or(file).to_owned();
        let parsed = object(info.path_id)
            .and_then(|data| TextAsset::parse(data, asset_path).ok())
            .and_then(|asset| MapConfig::parse(&asset.script, asset_path).ok());
        if let Some(config) = parsed {
            configs.insert(name, config);
        }
    }
    Ok((configs, plate))
}

/// The `PSX/FieldMapActorShadow` fragment for one texel, baked for a
/// reverse-subtract draw of rgb times alpha 1: `r0 = texel * vertexColor
/// * _Color`, killed when r, g and b times `r0.a` are all under 0.005,
/// else `2 * r0.a * r0.rgb` is subtracted.
pub fn shadow_texel(texel: [u8; 4], strength: f32) -> [u8; 4] {
    let alpha = f32::from(texel[3]) / 255.0 * SHADOW_VERTEX_ALPHA;
    let rgb = [0, 1, 2].map(|channel| f32::from(texel[channel]) / 255.0 * strength);
    if rgb.iter().all(|value| value * alpha < 0.005) {
        return [0; 4];
    }
    let out = rgb.map(|value| ((2.0 * alpha * value).min(1.0) * 255.0).round() as u8);
    [out[0], out[1], out[2], 255]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(lights: Vec<MapLight>, chars: Vec<MapChar>) -> MapConfig {
        MapConfig {
            attr: 0,
            version: 3,
            bg_no: 0,
            light_count: lights.len() as u8,
            light_use: u8::from(!lights.is_empty()),
            char_count: chars.len() as u8,
            char_use: u8::from(!chars.is_empty()),
            evt_count: 0,
            evt_use: 0,
            lights,
            chars,
            trailing: Vec::new(),
        }
    }

    fn light(kind: i8, floor: i8, color: i16, intensity: i16, radius: i8) -> MapLight {
        MapLight {
            kind,
            no: 0,
            shadow_intensity: intensity,
            shadow_radius: radius,
            color: [color, color, color, 0],
            floor: [floor, -1, -1, -1],
        }
    }

    fn char(geo_no: u16, color: i16, intensity: i16, radius: i8) -> MapChar {
        MapChar {
            geo_no,
            shadow_intensity: intensity,
            shadow_radius: radius,
            color: [color, color, color],
            shadow_z: 0,
        }
    }

    #[test]
    fn no_config_leaves_the_actor_untinted_and_shadowless() {
        assert_eq!(actor_light(None, 98, Some(0)), ActorLight::default());
    }

    #[test]
    fn a_floor_light_adds_to_the_char_light_and_its_shadow() {
        let config = config(
            vec![
                light(LIGHT_DEFAULT, -1, 8, 8, 1),
                light(LIGHT_FLOOR, 2, 16, 16, 2),
            ],
            vec![char(0xFFFF, 96, 24, 2)],
        );
        // Floor 2: char 96 + floor 16 = 112; shadow 24 + 16 = 40, radius 4.
        let lit = actor_light(Some(&config), 98, Some(2));
        assert_eq!(lit.tint, [224.0 / 255.0; 3]);
        assert_eq!(
            lit.shadow,
            Some(ShadowLook {
                strength: 80.0 / 255.0,
                half: [56.0, 48.0],
                offset: [0.0; 2],
            })
        );
        // Another floor takes the default light.
        let lit = actor_light(Some(&config), 98, Some(5));
        assert_eq!(lit.tint, [208.0 / 255.0; 3]);
        assert_eq!(lit.shadow.unwrap().strength, 64.0 / 255.0);
    }

    #[test]
    fn a_model_light_wins_over_the_default_char_and_no_char_light_means_no_shadow() {
        let config = config(
            Vec::new(),
            vec![char(98, 64, 8, 1), char(0xFFFF, 128, 8, 1)],
        );
        assert_eq!(
            actor_light(Some(&config), 98, None).tint,
            [128.0 / 255.0; 3]
        );
        let lights_only = super::tests::config(vec![light(LIGHT_DEFAULT, -1, 8, 8, 1)], Vec::new());
        let lit = actor_light(Some(&lights_only), 98, None);
        assert_eq!(lit.tint, [272.0 / 255.0; 3]);
        assert_eq!(lit.shadow, None);
    }

    #[test]
    fn shadow_texels_follow_the_shader_kill_and_doubling() {
        assert_eq!(
            shadow_texel([255, 255, 255, 255], 0.5),
            [153, 153, 153, 255]
        );
        assert_eq!(shadow_texel([255, 255, 255, 0], 1.0), [0; 4]);
        assert_eq!(shadow_texel([1, 1, 1, 255], 0.5), [0; 4]);
    }

    #[test]
    fn scripts_color_the_actor_and_shape_its_shadow() {
        let config = MapConfig {
            attr: 0,
            version: 3,
            bg_no: 0,
            light_count: 0,
            light_use: 0,
            char_count: 0,
            char_use: 0,
            evt_count: 0,
            evt_use: 0,
            lights: Vec::new(),
            chars: Vec::new(),
            trailing: Vec::new(),
        };
        let mut config = Some(config);
        apply_light_changes(
            &mut config,
            &[
                LightChange::Model {
                    model: -1,
                    color: [100, 100, 100],
                    shadow_intensity: 40,
                    shadow_scale: 4,
                },
                LightChange::Default {
                    kind: 1,
                    color: [10, 20, 30],
                    shadow_intensity: 0,
                    shadow_scale: 0,
                },
                LightChange::Default {
                    kind: 2,
                    color: [50, 0, 0],
                    shadow_intensity: 0,
                    shadow_scale: 0,
                },
            ],
        );
        let config = config.unwrap();
        assert_eq!((config.char_use, config.light_use), (1, 1));
        let plain = ActorShadowScript::default();
        let lit = actor_light_scripted(Some(&config), 7, None, &plain);
        assert_eq!(lit.tint[0], 2.0 * 110.0 / 255.0);
        assert_eq!(lit.shadow.unwrap().half, [56.0, 48.0]);
        // The ladder light replaces the default while climbing.
        let ladder = ActorShadowScript {
            attr: CHAR_ATTR_LADDER,
            ..plain
        };
        let lit = actor_light_scripted(Some(&config), 7, None, &ladder);
        assert_eq!(lit.tint[0], 2.0 * 150.0 / 255.0);
        // `CHRCOLOR` wins over the lights; `SHADOWOFF` removes the shadow.
        let scripted = ActorShadowScript {
            color: Some([64, 128, 255]),
            attr: CHAR_ATTR_SHADOW_OFF,
            ..plain
        };
        let lit = actor_light_scripted(Some(&config), 7, None, &scripted);
        assert_eq!(lit.tint, [2.0 * 64.0 / 255.0, 2.0 * 128.0 / 255.0, 2.0]);
        assert_eq!(lit.shadow, None);
        // `SHADOWSCALE`, `SHADOWAMP` and `SHADOWOFFSET` apply without a config.
        let shadowed = ActorShadowScript {
            scale: Some([8, 4]),
            amp: Some(50),
            offset: Some([3, -2]),
            ..plain
        };
        let lit = actor_light_scripted(None, 7, None, &shadowed);
        assert_eq!(
            lit.shadow,
            Some(ShadowLook {
                strength: 100.0 / 255.0,
                half: [112.0, 48.0],
                offset: [3.0, -2.0],
            })
        );
    }
}
