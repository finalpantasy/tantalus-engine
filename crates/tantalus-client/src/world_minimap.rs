//! The world map's minimap (`ff9.w_naviGetPos`, `WorldHUD.DisplayMinimap`):
//! a panel in the bottom-left corner with the land seen from above, a marker
//! for every place the party has been to and an arrow for the player.
//!
//! Before story point 5990 the map covers the first continent only
//! (`w_naviMapno` 0); after it, the whole world (1).

use tantalus_legacy::world::{RaycastMode, Vec3};
use tantalus_tools::world_play::WorldPlay;

use crate::battle_raster::Frame;

/// Panel width in pixels; the height follows the map's shape.
const PANEL_WIDTH: i32 = 150;
/// Gap from the window edge.
const MARGIN: i32 = 12;
/// World units per sample of the land picture.
const STEP: f32 = 8.0;
const SEA: [u8; 3] = [24, 52, 104];
const FRAME_COLOR: [u8; 3] = [236, 226, 190];
const PLACE: [u8; 3] = [255, 214, 64];
const PLAYER: [u8; 3] = [255, 64, 48];

/// Place markers (`ff9.w_naviLocationPos`): minimap number (0 = the first
/// continent's map, 1 = the whole world), location bit, world position in
/// 1/256 units. Location `n` is place `n + 1`.
const LOCATIONS: [(u8, u8, i32, i32); 80] = [
    (0, 0, 345560, -173564),
    (0, 1, 328680, -178910),
    (0, 2, 311680, -191323),
    (0, 3, 306501, -204700),
    (0, 4, 358057, -236117),
    (0, 5, 328239, -245345),
    (0, 6, 300090, -230918),
    (0, 7, 300090, -230918),
    (0, 8, 300090, -230918),
    (0, 9, 300090, -230918),
    (0, 10, 300090, -230918),
    (0, 11, 297056, -215824),
    (0, 12, 284585, -203240),
    (0, 13, 280253, -206433),
    (0, 14, 265584, -193337),
    (0, 15, 265584, -193337),
    (0, 16, 254112, -205731),
    (0, 17, 246186, -182059),
    (0, 18, 229548, -198479),
    (0, 19, 278526, -243814),
    (0, 20, 250721, -224068),
    (0, 21, 239319, -241340),
    (0, 22, 241248, -279572),
    (0, 23, 234496, -260825),
    (0, 24, 229342, -278220),
    (0, 25, 221494, -282188),
    (1, 0, 345560, -173564),
    (1, 1, 328680, -178910),
    (1, 2, 311680, -191323),
    (1, 3, 306501, -204700),
    (1, 4, 358057, -236117),
    (1, 5, 328239, -245345),
    (1, 6, 300090, -230918),
    (1, 7, 300090, -230918),
    (1, 8, 300090, -230918),
    (1, 9, 300090, -230918),
    (1, 10, 300090, -230918),
    (1, 11, 297056, -215824),
    (1, 12, 284585, -203240),
    (1, 13, 280253, -206433),
    (1, 14, 265584, -193337),
    (1, 15, 265584, -193337),
    (1, 16, 254112, -205731),
    (1, 17, 246186, -182059),
    (1, 18, 229548, -198479),
    (1, 19, 278526, -243814),
    (1, 20, 250721, -224068),
    (1, 21, 239319, -241340),
    (1, 22, 241248, -279572),
    (1, 23, 234496, -260825),
    (1, 24, 229342, -278220),
    (1, 25, 221494, -282188),
    (1, 26, 299041, -94130),
    (1, 27, 303345, -71006),
    (1, 28, 275329, -27154),
    (1, 29, 262598, -79454),
    (1, 30, 244288, -100007),
    (1, 31, 235576, -102928),
    (1, 32, 229037, -90135),
    (1, 33, 233101, -48649),
    (1, 34, 226757, -59696),
    (1, 35, 217668, -70533),
    (1, 36, 200097, -82107),
    (1, 37, 151941, -289631),
    (1, 38, 131348, -231368),
    (1, 39, 97345, -262526),
    (1, 40, 78285, -239115),
    (1, 41, 101014, -204347),
    (1, 42, 83497, -145686),
    (1, 43, 57147, -155460),
    (1, 44, 49042, -121464),
    (1, 45, 87500, -125280),
    (1, 46, 116381, -81633),
    (1, 47, 95358, -61173),
    (1, 48, 129014, -29732),
    (1, 54, 334274, -164988),
    (1, 55, 320846, -111720),
    (1, 56, 119880, -159015),
    (1, 57, 120775, -275601),
    (1, 58, 187061, -151788),
];

/// The world rectangle one minimap shows: `(x, z)` of its corner nearest
/// the origin and its size, with z running toward the south.
fn bounds(map: u8) -> (f32, f32, f32, f32) {
    if map == 0 {
        (645.12, 428.8, 1536.0 - 645.12, 1280.0 - 428.8)
    } else {
        (0.0, 0.0, 1536.0, 1280.0)
    }
}

/// `ff9.w_naviGetPos`: where a world position falls on the map, 0 to 1 from
/// the left and from the top.
pub fn navi_position(map: u8, x: f32, z: f32) -> (f32, f32) {
    let (left, top, width, height) = bounds(map);
    ((x - left) / width, (-z - top) / height)
}

/// The land picture of one map, built from the walk meshes.
struct Picture {
    map: u8,
    width: usize,
    height: usize,
    pixels: Vec<[u8; 3]>,
}

#[derive(Default)]
pub struct Minimap {
    picture: Option<Picture>,
}

impl Minimap {
    fn picture(&mut self, play: &WorldPlay) -> &Picture {
        let map = play.navi_map;
        if self.picture.as_ref().is_none_or(|p| p.map != map) {
            let (left, top, w, h) = bounds(map);
            let (width, height) = ((w / STEP) as usize, (h / STEP) as usize);
            let mode = RaycastMode::default();
            let mut pixels = Vec::with_capacity(width * height);
            for row in 0..height {
                for column in 0..width {
                    let x = left + (column as f32 + 0.5) * STEP;
                    let z = -(top + (row as f32 + 0.5) * STEP);
                    let hit = play.assets.walk.nwp_hit(Vec3::new(x, 40.0, z), &mode, None);
                    pixels.push(if hit.pno == 1000 && hit.height > 0.6 {
                        land(hit.height)
                    } else {
                        SEA
                    });
                }
            }
            self.picture = Some(Picture {
                map,
                width,
                height,
                pixels,
            });
        }
        self.picture.as_ref().expect("picture")
    }

    /// Draws the minimap over `frame`.
    pub fn draw(&mut self, frame: &mut Frame, play: &WorldPlay) {
        let picture = self.picture(play);
        let panel_w = PANEL_WIDTH;
        let panel_h = (panel_w as f32 * picture.height as f32 / picture.width as f32) as i32;
        let left = MARGIN;
        let top = frame.height as i32 - MARGIN - panel_h;
        frame.rect(
            left - 2,
            top - 2,
            panel_w + 4,
            panel_h + 4,
            FRAME_COLOR,
            1.0,
        );
        for y in 0..panel_h {
            for x in 0..panel_w {
                let px = (x as usize * picture.width / panel_w as usize).min(picture.width - 1);
                let py = (y as usize * picture.height / panel_h as usize).min(picture.height - 1);
                frame.rect(
                    left + x,
                    top + y,
                    1,
                    1,
                    picture.pixels[py * picture.width + px],
                    1.0,
                );
            }
        }
        let at = |x: f32, z: f32| {
            let (u, v) = navi_position(picture.map, x, z);
            (
                left + (u * panel_w as f32) as i32,
                top + (v * panel_h as f32) as i32,
            )
        };
        for &(map, location, x, z) in &LOCATIONS {
            if map != picture.map || play.visited >> location & 1 == 0 {
                continue;
            }
            let (px, py) = at(x as f32 / 256.0, z as f32 / 256.0);
            if (left..left + panel_w).contains(&px) && (top..top + panel_h).contains(&py) {
                frame.rect(px - 2, py - 2, 5, 5, [40, 28, 8], 1.0);
                frame.rect(px - 1, py - 1, 3, 3, PLACE, 1.0);
            }
        }
        // The player: a dot with a pointer along the facing. Yaw 0 faces
        // north (-z) and 180 south, as the model turns.
        let pos = play.walker.pos;
        let (px, py) = at(pos.x, pos.z);
        let yaw = play.walker.rot_y.to_radians();
        let (dx, dy) = (-yaw.sin(), -yaw.cos());
        for step in 1..=6 {
            let reach = step as f32;
            frame.rect(
                px + (dx * reach).round() as i32,
                py + (dy * reach).round() as i32,
                2,
                2,
                PLAYER,
                1.0,
            );
        }
        frame.rect(px - 2, py - 2, 5, 5, [255, 255, 255], 1.0);
        frame.rect(px - 1, py - 1, 3, 3, PLAYER, 1.0);
    }
}

/// Ground color by height: lowland green rising to rock.
fn land(height: f32) -> [u8; 3] {
    let t = (height / 60.0).clamp(0.0, 1.0);
    let low = [74.0, 128.0, 66.0];
    let high = [172.0, 158.0, 132.0];
    [0, 1, 2].map(|c| (low[c] + (high[c] - low[c]) * t).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_fall_in_the_map_like_navi_get_pos() {
        // The first continent's map starts at x 645.12 and z -428.8.
        let (u, v) = navi_position(0, 645.12, -428.8);
        assert!(u.abs() < 1e-6 && v.abs() < 1e-6);
        let (u, v) = navi_position(0, 1536.0, -1280.0);
        assert!((u - 1.0).abs() < 1e-6 && (v - 1.0).abs() < 1e-6);
        let (u, v) = navi_position(1, 768.0, -640.0);
        assert!((u - 0.5).abs() < 1e-6 && (v - 0.5).abs() < 1e-6);
    }

    #[test]
    fn every_place_marker_sits_on_its_map() {
        for &(map, _, x, z) in &LOCATIONS {
            let (u, v) = navi_position(map, x as f32 / 256.0, z as f32 / 256.0);
            assert!(
                (-0.05..=1.05).contains(&u) && (-0.05..=1.05).contains(&v),
                "{map} {x} {z}"
            );
        }
    }
}
