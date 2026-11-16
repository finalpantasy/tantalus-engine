//! Field background tile animations (`FieldMap.BgAnimationService` and the
//! `EBG_anim*` calls the `BGA*` opcodes make). Each animation cycles a list
//! of frames, every frame naming one overlay (tile block); the service shows
//! the current frame's overlay and hides the others by writing
//! `OverlayChange.active`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::world::OverlayChange;

/// `BGANIM_DEF.ANIM_FLAG`.
pub const FLAG_SINGLE_FRAME: u8 = 1;
pub const FLAG_ANIMATE: u8 = 2;
pub const FLAG_HAS_NOT_FINISHED: u8 = 4;
pub const FLAG_LOOP: u8 = 16;
pub const FLAG_PALINDROME: u8 = 32;
const START_PLAY: u8 = FLAG_ANIMATE | FLAG_HAS_NOT_FINISHED;
const CONTINUE_PLAY: u8 = FLAG_HAS_NOT_FINISHED | FLAG_LOOP;
/// `ANIM_FLAG.Modifiables`: the only bits `SetTileAnimationFlags` adds.
const MODIFIABLES: u8 = FLAG_LOOP | FLAG_PALINDROME;

/// One field animation as `BGANIM_DEF` holds it at run time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileAnimation {
    pub flags: u8,
    /// The field camera this animation belongs to.
    pub camera: u8,
    /// Frame index in 8.8 fixed point.
    pub current_frame: i32,
    pub frame_rate: i16,
    pub counter: u16,
    /// `(overlay index, wait ticks)` per frame; a negative wait stops.
    pub frames: Vec<(u8, i8)>,
}

impl TileAnimation {
    /// The state `BGANIM_DEF.ReadData` leaves after loading: the file's
    /// flags, frame, rate and counter are replaced by these.
    pub fn from_file(camera: u8, frames: Vec<(u8, i8)>) -> Self {
        Self {
            flags: FLAG_SINGLE_FRAME,
            camera,
            current_frame: 0,
            frame_rate: 256,
            counter: 0,
            frames,
        }
    }
}

fn set_active(overlays: &mut BTreeMap<u16, OverlayChange>, target: u8, active: bool) {
    overlays.entry(u16::from(target)).or_default().active = Some(active);
}

/// `BgAnimationService`: one field tick for every animation of the current
/// camera.
pub fn service(
    animations: &mut [TileAnimation],
    camera: u8,
    overlays: &mut BTreeMap<u16, OverlayChange>,
) {
    for animation in animations.iter_mut() {
        if animation.flags & FLAG_ANIMATE == 0
            || animation.flags & CONTINUE_PLAY == 0
            || animation.camera != camera
            || animation.frames.is_empty()
        {
            continue;
        }
        let count = animation.frames.len() as i32;
        let current = animation.current_frame >> 8;
        let Some(&(target, wait)) = usize::try_from(current)
            .ok()
            .and_then(|index| animation.frames.get(index))
        else {
            continue;
        };
        for &(frame_target, _) in &animation.frames {
            set_active(overlays, frame_target, false);
        }
        set_active(overlays, target, true);
        if wait < 0 {
            animation.frames[current as usize].1 = 0;
            animation.counter = 0;
            animation.flags &= !FLAG_HAS_NOT_FINISHED;
            continue;
        }
        animation.counter = animation.counter.wrapping_add(1);
        if i32::from(animation.counter) < i32::from(wait) {
            continue;
        }
        animation.counter = 0;
        animation.current_frame = animation
            .current_frame
            .wrapping_add(i32::from(animation.frame_rate));
        let next = animation.current_frame >> 8;
        let palindrome = animation.flags & FLAG_PALINDROME != 0;
        if next >= count {
            if palindrome {
                animation.current_frame = (count - 1) << 8;
                animation.frame_rate = animation.frame_rate.wrapping_neg();
            } else {
                animation.current_frame = 0;
            }
            animation.flags &= !FLAG_HAS_NOT_FINISHED;
        } else if next < 0 {
            if palindrome {
                animation.current_frame = 0;
                animation.frame_rate = animation.frame_rate.wrapping_neg();
            } else {
                animation.current_frame = (count - 1) << 8;
            }
            animation.flags &= !FLAG_HAS_NOT_FINISHED;
        }
    }
}

/// `EBG_animAnimate` (`RunTileAnimation`).
pub fn animate(animations: &mut [TileAnimation], index: i32, frame: i32) {
    if let Some(animation) = get(animations, index) {
        animation.flags |= FLAG_ANIMATE;
        animation.current_frame = frame << 8;
        animation.counter = 0;
    }
}

/// `EBG_animSetActive` (`SetTileAnimationActive`).
pub fn set_playing(animations: &mut [TileAnimation], index: i32, active: bool) {
    if let Some(animation) = get(animations, index) {
        if active {
            animation.flags |= START_PLAY;
        } else {
            animation.flags &= !START_PLAY;
        }
    }
}

/// `EBG_animSetFrameRate` (`SetTileAnimationRate`).
pub fn set_rate(animations: &mut [TileAnimation], index: i32, rate: i16) {
    if let Some(animation) = get(animations, index) {
        animation.frame_rate = rate;
    }
}

/// `EBG_animSetFrameWait` (`SetTileAnimationFrameWait`); the wait is a
/// signed byte.
pub fn set_frame_wait(animations: &mut [TileAnimation], index: i32, frame: i32, wait: i32) {
    if let Some(animation) = get(animations, index)
        && let Some(slot) = usize::try_from(frame)
            .ok()
            .and_then(|frame| animation.frames.get_mut(frame))
    {
        slot.1 = wait as i8;
    }
}

/// `EBG_animSetFlags` (`SetTileAnimationFlags`): adds loop and palindrome.
pub fn add_flags(animations: &mut [TileAnimation], index: i32, flags: i32) {
    if let Some(animation) = get(animations, index) {
        animation.flags |= (flags as u8) & MODIFIABLES;
    }
}

/// `EBG_animSetPlayRange` (`RunTileAnimationEx`).
pub fn set_range(animations: &mut [TileAnimation], index: i32, first: i32, last: i32) {
    if let Some(animation) = get(animations, index) {
        animation.flags |= START_PLAY;
        animation.current_frame = first << 8;
        animation.counter = 0;
        if let Some(slot) = usize::try_from(last)
            .ok()
            .and_then(|last| animation.frames.get_mut(last))
        {
            slot.1 = -1;
        }
        if (animation.frame_rate > 0 && last < first) || (animation.frame_rate < 0 && last > first)
        {
            animation.frame_rate = animation.frame_rate.wrapping_neg();
        }
    }
}

/// `EBG_animShowFrame` (`SetTileAnimationFrame`): shows one frame's overlay
/// and hides the rest; an out-of-range frame hides them all.
pub fn show_frame(
    animations: &mut [TileAnimation],
    index: i32,
    frame: i32,
    overlays: &mut BTreeMap<u16, OverlayChange>,
) {
    if let Some(animation) = get(animations, index) {
        for (position, &(target, _)) in animation.frames.iter().enumerate() {
            set_active(overlays, target, position as i32 == frame);
        }
        animation.flags = FLAG_SINGLE_FRAME;
    }
}

/// `EBG_animSetVisible` (`ResetTileAnimation`): stops the animation and
/// shows or hides its first frame's overlay.
pub fn reset(
    animations: &mut [TileAnimation],
    index: i32,
    visible: bool,
    overlays: &mut BTreeMap<u16, OverlayChange>,
) {
    if let Some(animation) = get(animations, index) {
        animation.current_frame = 0;
        animation.frame_rate = 256;
        animation.counter = 0;
        animation.flags = FLAG_SINGLE_FRAME;
        for (position, &(target, _)) in animation.frames.iter().enumerate() {
            set_active(overlays, target, visible && position == 0);
        }
    }
}

fn get(animations: &mut [TileAnimation], index: i32) -> Option<&mut TileAnimation> {
    usize::try_from(index)
        .ok()
        .and_then(|index| animations.get_mut(index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animation(frames: &[(u8, i8)]) -> Vec<TileAnimation> {
        vec![TileAnimation::from_file(0, frames.to_vec())]
    }

    fn visible(overlays: &BTreeMap<u16, OverlayChange>) -> Vec<u16> {
        overlays
            .iter()
            .filter(|(_, change)| change.active == Some(true))
            .map(|(&index, _)| index)
            .collect()
    }

    #[test]
    fn steps_at_rate_and_wraps() {
        let mut animations = animation(&[(10, 2), (11, 2), (12, 2)]);
        set_playing(&mut animations, 0, true);
        add_flags(&mut animations, 0, i32::from(FLAG_LOOP));
        animate(&mut animations, 0, 0);
        let mut overlays = BTreeMap::new();
        let mut shown = Vec::new();
        for _ in 0..8 {
            service(&mut animations, 0, &mut overlays);
            shown.push(visible(&overlays)[0]);
        }
        // Each frame shows for two ticks, then the cycle wraps to frame 0.
        assert_eq!(shown, [10, 10, 11, 11, 12, 12, 10, 10]);
        assert_eq!(overlays.len(), 3);
    }

    #[test]
    fn half_rate_holds_each_frame_twice_as_long() {
        let mut animations = animation(&[(1, 1), (2, 1)]);
        set_playing(&mut animations, 0, true);
        set_rate(&mut animations, 0, 128);
        let mut overlays = BTreeMap::new();
        let mut shown = Vec::new();
        for _ in 0..4 {
            service(&mut animations, 0, &mut overlays);
            shown.push(visible(&overlays)[0]);
        }
        assert_eq!(shown, [1, 1, 2, 2]);
    }

    #[test]
    fn palindrome_reverses_at_the_end() {
        let mut animations = animation(&[(1, 1), (2, 1), (3, 1)]);
        set_playing(&mut animations, 0, true);
        add_flags(&mut animations, 0, i32::from(FLAG_PALINDROME | FLAG_LOOP));
        let mut overlays = BTreeMap::new();
        let mut shown = Vec::new();
        for _ in 0..7 {
            service(&mut animations, 0, &mut overlays);
            shown.push(visible(&overlays)[0]);
        }
        assert_eq!(shown, [1, 2, 3, 3, 2, 1, 1]);
    }

    #[test]
    fn negative_wait_stops_the_animation() {
        let mut animations = animation(&[(1, 1), (2, 1), (3, 1)]);
        set_range(&mut animations, 0, 0, 1);
        let mut overlays = BTreeMap::new();
        let mut shown = Vec::new();
        for _ in 0..5 {
            service(&mut animations, 0, &mut overlays);
            shown.push(visible(&overlays)[0]);
        }
        assert_eq!(shown, [1, 2, 2, 2, 2]);
        assert_eq!(animations[0].flags & FLAG_HAS_NOT_FINISHED, 0);
        assert_eq!(animations[0].frames[1].1, 0);
    }

    #[test]
    fn deactivating_stops_and_other_cameras_are_skipped() {
        let mut animations = animation(&[(1, 1), (2, 1)]);
        set_playing(&mut animations, 0, true);
        let mut overlays = BTreeMap::new();
        service(&mut animations, 1, &mut overlays);
        assert!(overlays.is_empty());
        service(&mut animations, 0, &mut overlays);
        assert_eq!(visible(&overlays), [1]);
        set_playing(&mut animations, 0, false);
        service(&mut animations, 0, &mut overlays);
        service(&mut animations, 0, &mut overlays);
        assert_eq!(animations[0].current_frame >> 8, 1);
        assert_eq!(visible(&overlays), [1]);
    }

    #[test]
    fn show_frame_and_reset_set_overlays() {
        let mut animations = animation(&[(5, 1), (6, 1)]);
        let mut overlays = BTreeMap::new();
        show_frame(&mut animations, 0, 1, &mut overlays);
        assert_eq!(visible(&overlays), [6]);
        show_frame(&mut animations, 0, 255, &mut overlays);
        assert!(visible(&overlays).is_empty());
        reset(&mut animations, 0, true, &mut overlays);
        assert_eq!(visible(&overlays), [5]);
        reset(&mut animations, 0, false, &mut overlays);
        assert!(visible(&overlays).is_empty());
        // Out-of-range animation indices are ignored.
        animate(&mut animations, 9, 0);
    }
}
