//! Memoria's hand-tuned background layer fixes for widescreen
//! (`FieldMap.SceneServiceScroll`, the `InitializeWidescreenSupport` blocks,
//! and `EBG_overlayMove` for field 2755).
//!
//! Parallax and looping layers are drawn for a 320 wide view; on a wider view
//! Memoria shifts a few of them (`curX`, `curY`) and stretches some about the
//! layer's own origin (`transform.localScale`). `screen` is Memoria's
//! `ActualPsxScreenWidth` (220 rows times the aspect: 391 at 16:9, 513 at
//! 21:9), which the thresholds below read.

/// What a rule did to a layer besides moving it: a stretch about the layer's
/// origin, as `localScale` (x, y).
pub type Scale = [f32; 2];

/// `SceneServiceScroll`'s `Loop` block: 1651 and 1758 (Iifa roots).
/// `cur_x` is the layer's `curX`.
pub fn looped(map: i32, screen: f32, cur_x: &mut f32) -> Option<Scale> {
    match map {
        1651 | 1758 => {
            *cur_x = 200.0;
            if screen > 400.0 {
                *cur_x = 300.0;
                return Some([1.7, 1.0]);
            }
            None
        }
        _ => None,
    }
}

/// `SceneServiceScroll`'s `Parallax` block for layer `i` of `map`.
/// `cur` is the layer's `[curX, curY]` as the parallax just set it,
/// `camera_x` the camera's x (`mainCamera.localPosition.x`) and `scroll_x`
/// the layer's `scrollX`. Returns the stretch the case asks for, if any.
pub fn parallax(
    map: i32,
    i: usize,
    screen: f32,
    cur: &mut [f32; 2],
    camera_x: f32,
    scroll_x: i16,
) -> Option<Scale> {
    let wide = screen > 400.0;
    let mut scale = None;
    let uniform = |s: f32| Some([s, s]);
    match map {
        312 if wide => cur[0] = 0.0,
        805 | 808 if wide => {
            scale = uniform(1.3);
            cur[0] = 0.0;
        }
        908 | 1908 if i == 14 && wide => cur[0] = 0.0,
        1108 => cur[0] = 0.0, // Clayra temple light
        1651 | 1758 => {
            // Iifa roots
            scale = uniform(1.1);
            cur[0] = -8.0;
        }
        1657 => cur[0] = camera_x * (f32::from(scroll_x) / 256.0) - 0.25,
        1660 => cur[0] = 0.0,
        2251 if wide => cur[0] = 0.0,
        2252 if screen >= 480.0 => cur[0] = 0.0,
        2600 if (i > 0 && i <= 7) || i == 13 => {
            // 464/416
            scale = uniform(1.14);
            cur[0] -= 24.0;
            if screen >= 464.0 {
                cur[0] = -12.0;
            }
        }
        2602 => {
            // 384/328
            scale = uniform(1.05);
            cur[0] = 28.0;
        }
        2604 if screen >= 448.0 => {
            // 512/448
            scale = uniform(1.15);
            cur[0] = 0.0;
        }
        2605 => {
            // 400/368
            scale = uniform(1.1);
            cur[0] -= 16.0;
            if wide {
                scale = uniform(1.45);
                cur[0] = -16.0;
            }
            if screen >= 500.0 {
                scale = uniform(1.75);
                cur[0] = -16.0;
            }
        }
        2606 => cur[0] = camera_x * (f32::from(scroll_x) / 256.0),
        2607 => {
            // 416/400
            scale = uniform(1.05);
            cur[0] -= 8.0;
            cur[1] -= 8.0;
            if wide {
                scale = uniform(1.1);
                cur[0] = 0.0;
            }
        }
        2651 if i == 3 || i == 4 => {
            // The parallax text (other layers) is left alone.
            scale = uniform(1.2);
            cur[0] -= 56.0;
            cur[1] -= 16.0;
            if wide {
                scale = uniform(1.5);
                cur[0] -= 70.0;
            }
        }
        2660 => {
            // 536/528
            scale = uniform(1.02);
            cur[0] -= 8.0;
            if wide {
                scale = uniform(1.15);
                cur[0] = 0.0;
            }
        }
        2851 => {
            cur[1] -= 12.0;
            if screen >= 384.0 {
                if i == 8 {
                    cur[0] = 128.0;
                }
                if i == 9 {
                    cur[0] = 80.0;
                }
            }
        }
        2916 => {
            scale = uniform(1.02);
            cur[0] -= 4.0;
            if wide {
                cur[0] = -4.0;
            }
        }
        2922 if screen > 364.0 && (i < 4 || i == 8 || i == 9) => cur[0] = 0.0,
        2923 => {
            if wide && (i == 2 || (4..=13).contains(&i) || i == 19) {
                cur[0] = 48.0;
            }
            if wide && (i == 1 || i == 3) {
                cur[0] = 0.0;
            }
        }
        _ => {}
    }
    scale
}

/// `FieldMap.EBG_overlayMove`'s widescreen case: field 2755 layer 5 moving
/// by `dx == 1` moves by 0.75 and is stretched 1.1. Returns the new `dx` and
/// the stretch.
pub fn overlay_move(map: i32, overlay: usize, dx: f32) -> Option<(f32, Scale)> {
    (map == 2755 && overlay == 5 && dx == 1.0).then_some((0.75, [1.1, 1.1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_cases_follow_memoria() {
        let mut cur = [10.0, 0.0];
        // 16:9 (391) is wide enough for 2605's first stretch only.
        assert_eq!(parallax(2605, 0, 391.0, &mut cur, 0.0, 0), Some([1.1; 2]));
        assert_eq!(cur, [-6.0, 0.0]);
        let mut cur = [10.0, 0.0];
        assert_eq!(parallax(2605, 0, 513.0, &mut cur, 0.0, 0), Some([1.75; 2]));
        assert_eq!(cur, [-16.0, 0.0]);
        let mut cur = [5.0, 5.0];
        assert_eq!(parallax(2851, 8, 391.0, &mut cur, 0.0, 0), None);
        assert_eq!(cur, [128.0, -7.0]);
        let mut cur = [5.0, 5.0];
        assert_eq!(parallax(312, 0, 391.0, &mut cur, 0.0, 0), None);
        assert_eq!(cur, [5.0, 5.0]);
    }

    #[test]
    fn loop_and_move_cases() {
        let mut x = 0.0;
        assert_eq!(looped(1651, 391.0, &mut x), None);
        assert_eq!(x, 200.0);
        assert_eq!(looped(1758, 513.0, &mut x), Some([1.7, 1.0]));
        assert_eq!(x, 300.0);
        assert_eq!(overlay_move(2755, 5, 1.0), Some((0.75, [1.1, 1.1])));
        assert_eq!(overlay_move(2755, 4, 1.0), None);
    }
}
