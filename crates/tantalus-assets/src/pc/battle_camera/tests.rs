use super::*;

/// One set, one variant: at frame 1 the eye sits 1280 behind the origin
/// (key anchor 0, pitch 0, yaw 0) and looks at it.
fn simple_block() -> CameraBlock {
    let mut bytes = vec![2, 0];
    // Set at 2: header (variant 0), its offset (6), the records offset.
    bytes.extend([1, 0, 6, 0, 0, 0]);
    // Frame 1, eye and target keys (kind 1 each).
    bytes.extend([1, 0, 9, 0]);
    bytes.extend([0, 0x80, 0, 0, 0x00, 0x05]);
    bytes.extend([0, 0x80, 0, 0, 0, 0]);
    bytes.extend([0, 0]);
    CameraBlock { bytes }
}

#[test]
fn a_still_key_looks_from_behind_at_the_origin() {
    let mut script = CameraScript::new(simple_block(), 1);
    script.start_plunge(0);
    let camera = script.update(&CameraWorld::default()).expect("a camera");
    assert_eq!(script.eye_and_target(), ([0, 0, -1280], [0, 0, 0]));
    assert_eq!(camera.rotation, [[4096, 0, 0], [0, 4096, 0], [0, 0, 4096]]);
    assert_eq!(camera.translation, [0, 0, 1280]);
    assert_eq!(camera.h, 512);
    assert!(!script.running());
}

#[test]
fn a_moving_key_blends_linearly() {
    // Frame 1: both tracks at distance 1280; frame 2: the eye moves to
    // 2560 over 4 frames (kind 3, linear).
    let mut bytes = vec![2, 0, 1, 0, 6, 0, 0, 0];
    bytes.extend([1, 0, 9, 0]);
    bytes.extend([0, 0x80, 0, 0, 0x00, 0x05]);
    bytes.extend([0, 0x80, 0, 0, 0, 0]);
    bytes.extend([2, 0, 3, 0]);
    bytes.extend([0, 0x80, 0, 0, 0x00, 0x0A]);
    bytes.extend([4, 0, 0, 0]);
    bytes.extend([0, 0]);
    let mut script = CameraScript::new(CameraBlock { bytes }, 1);
    script.start_plunge(0);
    let world = CameraWorld::default();
    let mut z = Vec::new();
    for _ in 0..6 {
        script.update(&world);
        z.push(script.eye_and_target().0[2]);
    }
    assert_eq!(z, [-1280, -1600, -1920, -2240, -2560, -2560]);
}

/// Runs only with a real install (`FFIX_RAW17_TH_E001` names the scene's
/// `.raw17`): the opening of battle 336 ends and hands over to set 1.
#[test]
fn real_opening_reaches_the_default_camera() {
    let Some(path) = std::env::var_os("FFIX_RAW17_TH_E001") else {
        return;
    };
    let raw = std::fs::read(path).unwrap();
    let block = CameraBlock::from_raw17(&raw).unwrap();
    assert_eq!(block.sets(), 9);
    let mut script = CameraScript::new(block, 7);
    script.start_plunge(0);
    let world = CameraWorld::default();
    let mut frames = 0;
    while script.phase == 0 && frames < 400 {
        assert!(script.update(&world).is_some() || frames == 0);
        frames += 1;
    }
    assert_eq!(frames, 112);
}
