//! Status effects drawn as SPS sprite strips over the units in battle (Poison, Venom, Sleep,
//! Heat, Freeze, Reflect, Blind, Berserk): `btl2d.StatusUpdateVisuals` places each afflicted
//! unit's effect at its icon position (`btl2d.GetIconPosition`), and `BattleSPSSystem` draws
//! it as `SPSEffect._GenerateSPSMesh` does in battle mode (`gMode` 2): a quad mesh facing the
//! camera, scaled by its distance to the camera (`useBattleFactors`).

use crate::battle_raster::{BattleCamera, Blend, Frame, ScreenCorner, Texture};
use bevy::math::Vec3;
use tantalus_assets::pc::battle_status_sps::{STATUS_SPS, StatusIcons, StatusSps};
use tantalus_legacy::battle::status;
use tantalus_sfx::player::Bone;

/// `SPSEffect` battle factors: distances are scaled by this before the prototype's factors.
const DISTANCE_UNIT: f32 = 0.000_259_551_48;

/// One afflicted unit: where it is and what it is under.
pub struct Afflicted<'a> {
    /// `stat.cur`.
    pub status: u64,
    pub icons: StatusIcons,
    /// The posed model's bones (PSX space, y down).
    pub bones: &'a [Option<Bone>],
    /// Degrees about y (`rot.eulerAngles.y`).
    pub turn: f32,
}

/// The world point (y up) of icon position `index` on `unit` (`btl2d.GetIconPosition`):
/// the bone, raised `-icon_y << 4` and moved `icon_z << 4` along the facing.
fn icon_point(unit: &Afflicted<'_>, index: usize) -> Option<Vec3> {
    let bone = unit
        .bones
        .get(usize::from(unit.icons.bone[index]))?
        .as_ref()?;
    let at = Vec3::new(bone.t[0] as f32, -bone.t[1] as f32, bone.t[2] as f32);
    let dy = f32::from(i16::from(unit.icons.y[index]) << 4);
    let dz = f32::from(i16::from(unit.icons.z[index]) << 4);
    let angle = unit.turn.to_radians();
    Some(at + Vec3::new(dz * angle.sin(), -dy, dz * angle.cos()))
}

/// Whether `status` shows its SPS on a unit (`StatusUpdateVisuals`: not on the dead).
fn shows(status: u64) -> bool {
    status & status::DEATH == 0
}

/// Draws every afflicted unit's status SPS onto `frame`. `tick` is the battle's 2D timer
/// (`btl2d_work_set.Timer`), which sets each effect's frame.
pub fn draw(
    frame: &mut Frame,
    camera: &BattleCamera,
    library: &[Option<StatusSps>],
    units: &[Afflicted<'_>],
    tick: u64,
) {
    let eye = camera.view.inverse().transform_point3(Vec3::ZERO);
    let rotation = camera.view.inverse();
    let right = rotation.transform_vector3(Vec3::X).normalize_or_zero();
    let down = rotation.transform_vector3(-Vec3::Y).normalize_or_zero();
    for unit in units {
        if !shows(unit.status) {
            continue;
        }
        for &(bit, prototype, attach) in &STATUS_SPS {
            if unit.status & (1 << bit) == 0 {
                continue;
            }
            let Some(Some(sps)) = library.get(prototype) else {
                continue;
            };
            let Some(center) = icon_point(unit, attach) else {
                continue;
            };
            draw_one(frame, camera, sps, center, (eye, right, down), tick);
        }
    }
}

/// `SPSEffect.LoadSPS`, `_GenerateSPSPrims` and `_GenerateSPSMesh` for one effect at
/// `center` (world, y up).
fn draw_one(
    frame: &mut Frame,
    camera: &BattleCamera,
    sps: &StatusSps,
    center: Vec3,
    (eye, right, down): (Vec3, Vec3, Vec3),
    tick: u64,
) {
    let file = &sps.file;
    let frames = file.frames.len().max(1);
    // `UpdateBtlStatus`: `curFrame = Timer << 4`, read back as `curFrame >> 4`.
    let Some(prims) = file.frames.get(tick as usize % frames) else {
        return;
    };
    let distance = eye.distance(center);
    // `_GenerateSPSPrims`: sprite offsets shrink with distance (never under a third).
    let spread =
        (1.0 / (distance * sps.prototype.battle_distance * DISTANCE_UNIT).max(1.0)).max(0.33);
    // `_GenerateSPSMesh`: `scale / 4096` (one) times the distance, at most 3.
    let size = (distance * sps.prototype.battle_scale * DISTANCE_UNIT).min(3.0);
    let (texel_w, texel_h) = file.texel_size();
    let (area_min, area_size) = sps.texel_area();
    let texture = Texture {
        width: sps.width,
        height: sps.height,
        rgba: &sps.rgba,
        clamp: true,
    };
    let blend = match sps.prototype.abr {
        1 | 3 => Blend::Add,
        2 => Blend::Subtract,
        _ => Blend::Alpha(255),
    };
    for prim in prims {
        let x = (prim.x as f32 * spread) as i32;
        let y = (prim.y as f32 * spread) as i32;
        let (x0, x1) = (x - file.half_width, x + file.half_width);
        let (y0, y1) = (y - file.half_height, y + file.half_height);
        // The quad faces the camera: local x along the screen's right, local y down it.
        let corner = |lx: i32, ly: i32| {
            let world = center + (right * lx as f32 + down * ly as f32) * size;
            frame.project(camera, camera.to_view(world.to_array()))
        };
        let (Some(a), Some(b), Some(c), Some(d)) = (
            corner(x0, y0),
            corner(x1, y0),
            corner(x0, y1),
            corner(x1, y1),
        ) else {
            continue;
        };
        let uv = |u: i32, v: i32| {
            [
                (u as f32 - area_min[0]) / area_size[0],
                (v as f32 - area_min[1]) / area_size[1],
            ]
        };
        let (u0, v0) = (i32::from(prim.u), i32::from(prim.v));
        let (u1, v1) = (u0 + texel_w, v0 + texel_h);
        let color = crate::sps_view::sprite_color(prim.color, 128, -1, -1);
        let at = |xy: [f32; 2], uv: [f32; 2]| ScreenCorner { xy, uv, color };
        // `_indices`: (0, 1, 2) and (1, 3, 2).
        frame.draw_screen(
            [at(a, uv(u0, v0)), at(b, uv(u1, v0)), at(c, uv(u0, v1))],
            Some(texture),
            blend,
        );
        frame.draw_screen(
            [at(b, uv(u1, v0)), at(d, uv(u1, v1)), at(c, uv(u0, v1))],
            Some(texture),
            blend,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_points_rise_from_the_bone_and_follow_the_facing() {
        let mut bones = vec![None; 9];
        bones[8] = Some(Bone {
            r: [0; 9],
            t: [100, -700, 50],
        });
        let unit = Afflicted {
            status: status::POISON,
            icons: StatusIcons {
                bone: [8; 6],
                y: [-10, 0, 0, 0, 0, 0],
                z: [-1, 0, 0, 0, 0, 0],
            },
            bones: &bones,
            turn: 0.0,
        };
        // Bone 8 at (100, 700 up, 50), 160 higher and 16 back along +z facing.
        let at = icon_point(&unit, 0).unwrap();
        assert_eq!(at, Vec3::new(100.0, 860.0, 34.0));
        assert!(shows(status::POISON));
        assert!(!shows(status::POISON | status::DEATH));
    }
}
