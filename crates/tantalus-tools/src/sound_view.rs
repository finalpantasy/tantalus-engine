//! Where a field sound sits for the listener: `FF9Snd.FF9FieldSoundGetPositionVolume`
//! (`FF9Snd.cs:1729`). The point goes through the field camera (the GTE projection the
//! background uses), and its screen column becomes the pan and its depth the volume.

use tantalus_contracts::LegacyFieldCamera;

/// `FF9Snd.FF9_SCREEN_WIDTH`.
const SCREEN_WIDTH: i32 = 320;
/// Pan and volume with no camera (centered, full): what a sound without a position gets.
pub const CENTERED: (i32, i32) = (128, 127);

/// `FF9FieldSoundGetPositionVolume` for a field-world point: `(pos, vol)`, `pos` 0 (left) to
/// 255 (right) with 128 centered, `vol` 0 to 127.
pub fn position_volume(camera: &LegacyFieldCamera, point: [f32; 3]) -> (i32, i32) {
    let rotation =
        |row: usize, column: usize| f32::from(camera.rotation[row * 3 + column]) / 4096.0;
    let translation = [
        camera.translation.x as f32,
        camera.translation.y as f32,
        camera.translation.z as f32,
    ];
    // `RTPT_POS` flips y on the way in and back out.
    let v = [point[0], -point[1], point[2]];
    let mut out = [0.0f32; 3];
    for (row, value) in out.iter_mut().enumerate() {
        *value = rotation(row, 0) * v[0]
            + rotation(row, 1) * v[1]
            + rotation(row, 2) * v[2]
            + translation[row];
    }
    let depth = out[2].abs();
    if depth < f32::EPSILON {
        return CENTERED;
    }
    // `FieldMap.offset`, added to every projected point (`FieldMap.cs:393`).
    let offset_x = f32::from(camera.center_offset[0]) + f32::from(camera.size[0]) / 2.0 - 160.0;
    let screen_x = (out[0] * f32::from(camera.projection) / depth + offset_x) as i16;
    let mut x = i32::from(screen_x);
    let ordering = ((depth as i64) / 4 + i64::from(camera.depth_offset)).clamp(0, 4095);
    let near = (ordering * 128 / 4096) as i32;
    let spread = (128 - near) * 2;
    let pan = if (0..SCREEN_WIDTH).contains(&x) {
        near + x * spread / SCREEN_WIDTH
    } else if x < 0 {
        x = x.max(-SCREEN_WIDTH);
        (x + SCREEN_WIDTH) * near / SCREEN_WIDTH
    } else {
        x = x.min(639);
        255 - near + (x - SCREEN_WIDTH) * near / SCREEN_WIDTH
    };
    let volume = ((4096 - ordering) * 127 / 4096) as i32;
    (pan & 255, volume)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_contracts::Point3;

    fn camera() -> LegacyFieldCamera {
        // Looking down +z from the origin: identity rotation, projection 400, frame 320 wide.
        LegacyFieldCamera {
            projection: 400,
            rotation: [4096, 0, 0, 0, 4096, 0, 0, 0, 4096],
            translation: Point3 { x: 0, y: 0, z: 0 },
            center_offset: [0, 0],
            size: [320, 224],
            viewport_x: [0, 0],
            viewport_y: [0, 0],
            depth_offset: 0,
        }
    }

    #[test]
    fn a_far_sound_is_quieter() {
        let near = position_volume(&camera(), [0.0, 0.0, 400.0]);
        let far = position_volume(&camera(), [0.0, 0.0, 12_000.0]);
        assert!(far.1 < near.1, "{far:?} vs {near:?}");
    }

    #[test]
    fn a_sound_to_the_left_pans_left_of_one_to_the_right() {
        let left = position_volume(&camera(), [-300.0, 0.0, 1000.0]);
        let right = position_volume(&camera(), [300.0, 0.0, 1000.0]);
        assert!(left.0 < right.0, "{left:?} vs {right:?}");
    }
}
