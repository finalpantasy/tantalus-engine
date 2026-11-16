//! Field background animation: event-VM host events applied to the pure
//! per-tick state, plus the install checks behind `tantalusctl`
//! (`--field-animation-frames`, `--field-animation-check`).

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tantalus_assets::pc::field_animation::{
    FieldAnimation, FieldAnimationError, WalkmeshAnimation, anim_flag, rasterize_state,
};
use tantalus_assets::pc::field_composition::{FieldVisit, build_layer_plan, visit_bundle_fields};
use tantalus_assets::pc::ui_atlas::encode_png_rgba;
use tantalus_legacy::HostEvent;

/// What [`FieldAnimationHost::apply`] did with an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventEffect {
    Applied,
    /// A walkmesh event arrived and no walkmesh is loaded.
    NoWalkmesh,
    /// Not a background or walkmesh animation event.
    NotAnimation,
}

/// Background animation state plus the optional walkmesh state of one field.
pub struct FieldAnimationHost {
    pub background: FieldAnimation,
    pub walkmesh: Option<WalkmeshAnimation>,
}

impl FieldAnimationHost {
    /// One field frame for both states.
    pub fn tick(&mut self) -> Result<(), FieldAnimationError> {
        self.background.tick()?;
        if let Some(walkmesh) = &mut self.walkmesh {
            walkmesh.tick()?;
        }
        Ok(())
    }

    /// Applies one event-VM host event with the engine's argument handling
    /// (`EventEngine.DoEventCode`): flags are non-zero tests, narrow casts
    /// wrap as in C#.
    pub fn apply(&mut self, event: &HostEvent) -> Result<EventEffect, FieldAnimationError> {
        let background = &mut self.background;
        match event {
            HostEvent::SetTileBlockActive { block, active } => {
                background.overlay_set_active(*block as i64, *active)?
            }
            HostEvent::DefineTileViewport {
                viewport,
                x,
                y,
                width,
                height,
            } => background.overlay_define_viewport(*viewport as i64, [*x, *y], [*width, *height]),
            HostEvent::SetTileViewportAlpha { viewport, alpha } => {
                background.overlay_define_viewport_alpha(*viewport as i64, *alpha)?
            }
            HostEvent::SetTileCamera { overlay, camera } => {
                background.overlay_set_viewport(*overlay as i64, *camera as i64)?
            }
            HostEvent::SetTileLoop {
                block,
                enabled,
                x_speed,
                y_speed,
            } => background.overlay_set_loop(*block as i64, *enabled != 0, [*x_speed, *y_speed])?,
            HostEvent::SetTileLoopType {
                overlay,
                screen_anchored,
            } => background.overlay_set_loop_type(*overlay as i64, *screen_anchored)?,
            HostEvent::SetTileParallax {
                block,
                enabled,
                x_speed,
                y_speed,
            } => background.overlay_set_parallax(
                *block as i64,
                *enabled != 0,
                [*x_speed, *y_speed],
            )?,
            HostEvent::SetTileOrigin { block, x, y } => {
                background.overlay_set_origin(*block as i64, *x as f32, *y as f32)?
            }
            HostEvent::MoveTile { overlay, offset } => background.overlay_move(
                *overlay as i64,
                offset[0] as f32,
                offset[1] as f32,
                offset[2],
            )?,
            HostEvent::SetTileScrollWithOffset {
                overlay,
                enabled,
                speed,
                offset,
                is_x_offset,
            } => background.overlay_set_scroll_with_offset(
                *overlay as i64,
                *enabled != 0,
                *speed,
                *offset,
                *is_x_offset != 0,
            )?,
            HostEvent::RunTileAnimation {
                animation,
                start_frame,
            } => background.anim_animate(*animation as i64, *start_frame)?,
            HostEvent::SetTileAnimationActive { animation, active } => {
                background.anim_set_active(*animation as i64, *active)?
            }
            HostEvent::ResetTileAnimation {
                animation,
                show_first_frame,
            } => background.anim_set_visible(*animation as i64, *show_first_frame != 0)?,
            HostEvent::SetTileAnimationRate {
                animation,
                frame_rate,
            } => background.anim_set_frame_rate(*animation as i64, *frame_rate as i32)?,
            HostEvent::SetTileAnimationFrameWait {
                animation,
                frame,
                wait,
            } => background.anim_set_frame_wait(*animation as i64, *frame as i64, *wait)?,
            HostEvent::SetTileAnimationFlags { animation, flags } => {
                background.anim_set_flags(*animation as i64, *flags)?
            }
            HostEvent::SetTileAnimationRange {
                animation,
                first_frame,
                last_frame,
            } => background.anim_set_play_range(
                *animation as i64,
                *first_frame,
                *last_frame as i64,
            )?,
            HostEvent::SetTileAnimationFrame { animation, frame } => {
                background.anim_show_frame(*animation as i64, *frame as i64)?
            }
            walkmesh_event => return self.apply_walkmesh(walkmesh_event),
        }
        Ok(EventEffect::Applied)
    }

    fn apply_walkmesh(&mut self, event: &HostEvent) -> Result<EventEffect, FieldAnimationError> {
        if !matches!(
            event,
            HostEvent::SetPathFloorActive { .. }
                | HostEvent::SetTriangleActive { .. }
                | HostEvent::SetSimulationFloor { .. }
                | HostEvent::SetSimulationFrameRate { .. }
                | HostEvent::SetSimulationActive { .. }
                | HostEvent::SetSimulationFlags { .. }
                | HostEvent::SetSimulationAlgorithm { .. }
                | HostEvent::SetSimulationDelta { .. }
                | HostEvent::SetSimulationAxis { .. }
                | HostEvent::ShowNavigationAnimationFrame { .. }
        ) {
            return Ok(EventEffect::NotAnimation);
        }
        let Some(walkmesh) = self.walkmesh.as_mut() else {
            return Ok(EventEffect::NoWalkmesh);
        };
        match event {
            HostEvent::SetPathFloorActive { floor, active } => {
                // `BGI_floorSetActive(UInt32, UInt32)`: `isActive > 0u`.
                walkmesh.floor_set_active(*floor as u32, (*active as u32) > 0);
            }
            HostEvent::SetTriangleActive { triangle, active } => {
                walkmesh.triangle_set_active(*triangle, *active)?
            }
            HostEvent::SetSimulationFloor { simulation, floor } => {
                walkmesh.simulation_set_floor(*simulation, *floor)?
            }
            HostEvent::SetSimulationFrameRate {
                simulation,
                frame_rate,
            } => walkmesh.simulation_set_frame_rate(*simulation, *frame_rate)?,
            HostEvent::SetSimulationActive { simulation, active } => {
                walkmesh.simulation_set_active(*simulation as u32, *active != 0)?
            }
            HostEvent::SetSimulationFlags { simulation, flags } => {
                walkmesh.simulation_set_flags(*simulation as u32, *flags as u32)?
            }
            HostEvent::SetSimulationAlgorithm {
                simulation,
                algorithm,
            } => walkmesh.simulation_set_algorithm(*simulation as u32, *algorithm as u32)?,
            HostEvent::SetSimulationDelta { simulation, delta } => {
                walkmesh.simulation_set_delta(*simulation as u32, delta[0], delta[1])?
            }
            HostEvent::SetSimulationAxis { simulation, axis } => {
                walkmesh.simulation_set_axis(*simulation as u32, *axis as u32)?
            }
            HostEvent::ShowNavigationAnimationFrame { animation, frame } => {
                walkmesh.animation_show_frame(*animation as u32, *frame as u32);
            }
            _ => unreachable!("filtered above"),
        }
        Ok(EventEffect::Applied)
    }
}

fn hash_hex(rgba: &[u8]) -> String {
    let digest = Sha256::digest(rgba);
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Starts every animation of a field at tick 0 (`BGAACTIVE`) and returns the
/// state after `ticks` ticks. This is the offline choice the tools make; the
/// real start comes from the field scripts.
fn run_all_animations(
    scene: &tantalus_assets::pc::bgs::BgsFile,
    ticks: u32,
) -> Result<FieldAnimation, String> {
    let mut state = FieldAnimation::new(scene).map_err(|error| error.to_string())?;
    for animation in 0..state.animations.len() {
        state
            .anim_set_active(animation as i64, true)
            .map_err(|error| error.to_string())?;
    }
    for _ in 0..ticks {
        state.tick().map_err(|error| error.to_string())?;
    }
    Ok(state)
}

fn find_bundles(path: &Path) -> Vec<PathBuf> {
    if path.is_dir() {
        (11..=19)
            .map(|n| path.join(format!("p0data{n}.bin")))
            .filter(|bundle| bundle.exists())
            .collect()
    } else {
        vec![path.to_path_buf()]
    }
}

/// `--field-animation-frames <field> <tick> <out.png>`: every animation of the
/// field started at tick 0, the field advanced `tick` ticks, one camera drawn.
pub fn run_field_animation_frames(
    path: &Path,
    field: &str,
    ticks: u32,
    camera: u8,
    out: &Path,
) -> Result<(), String> {
    let mut result: Option<Result<String, String>> = None;
    let mut load_problem: Option<String> = None;
    for bundle in find_bundles(path) {
        visit_bundle_fields(
            &bundle,
            Some(field),
            |visit: FieldVisit<'_>| {
                result = Some((|| {
                    let plan = build_layer_plan(
                        visit.scene,
                        [visit.atlas.width, visit.atlas.height],
                        visit.atlas.tile_size,
                        visit.atlas.padding,
                    )
                    .map_err(|error| error.to_string())?;
                    let camera_plan = plan
                        .cameras
                        .iter()
                        .find(|c| c.camera == camera)
                        .ok_or_else(|| format!("{field} has no camera {camera}"))?;
                    let mut state = run_all_animations(visit.scene, 0)?;
                    state.set_camera(camera);
                    for _ in 0..ticks {
                        state.tick().map_err(|error| error.to_string())?;
                    }
                    let raster = rasterize_state(camera_plan, &visit.atlas, &state)
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| format!("camera {camera} of {field} has no tiles"))?;
                    std::fs::write(
                        out,
                        encode_png_rgba(raster.width, raster.height, &raster.rgba),
                    )
                    .map_err(|error| format!("could not write {}: {error}", out.display()))?;
                    Ok(format!(
                        "{field} tick {ticks} camera {camera}: {}x{}, {} tiles, hash {} -> {}",
                        raster.width,
                        raster.height,
                        raster.tiles_drawn,
                        hash_hex(&raster.rgba),
                        out.display()
                    ))
                })());
            },
            |name, reason| load_problem = Some(format!("{name}: {reason}")),
        )
        .map_err(|error| error.to_string())?;
        if let Some(problem) = load_problem.take() {
            result = Some(Err(problem));
        }
        if result.is_some() {
            break;
        }
    }
    match result {
        Some(Ok(line)) => {
            println!("{line}");
            Ok(())
        }
        Some(Err(error)) => Err(error),
        None => Err(format!(
            "field `{field}` was not found under {}",
            path.display()
        )),
    }
}

#[derive(Debug, Serialize)]
struct AnimationCheck {
    animation: usize,
    camera: u8,
    frames: usize,
    visited_frames: Vec<usize>,
    /// Ticks until `HasNotFinished` cleared; `None` if the cycle never ended.
    cycle_ticks: Option<u32>,
    tick_bound: u32,
    /// Tiles of the frame overlays that fall outside the atlas.
    out_of_atlas_tiles: usize,
    errors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct FieldCheck {
    field: String,
    bundle: String,
    animations: usize,
    frames: usize,
    /// `[tick, camera-raster hash]` at the start, at the first shown change
    /// and at the end of animation 0's cycle.
    frame_hashes: Vec<(u32, String)>,
    animation_checks: Vec<AnimationCheck>,
    failures: Vec<String>,
    passed: bool,
}

fn check_field(visit: &FieldVisit<'_>) -> FieldCheck {
    let scene = visit.scene;
    let mut check = FieldCheck {
        field: visit.field.to_owned(),
        bundle: visit.bundle.to_owned(),
        animations: scene.animations.len(),
        frames: scene.animations.iter().map(|a| a.frames.len()).sum(),
        frame_hashes: Vec::new(),
        animation_checks: Vec::new(),
        failures: Vec::new(),
        passed: false,
    };
    let plan = match build_layer_plan(
        scene,
        [visit.atlas.width, visit.atlas.height],
        visit.atlas.tile_size,
        visit.atlas.padding,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            check.failures.push(error.to_string());
            return check;
        }
    };
    let overlay_atlas_misses = |overlay: usize| -> usize {
        plan.cameras
            .iter()
            .flat_map(|c| &c.layers)
            .filter(|l| l.overlay_index as usize == overlay)
            .flat_map(|l| &l.tiles)
            .filter(|t| !t.atlas_in_bounds)
            .count()
    };
    for (number, animation) in scene.animations.iter().enumerate() {
        let mut entry = AnimationCheck {
            animation: number,
            camera: animation.camera,
            frames: animation.frames.len(),
            visited_frames: Vec::new(),
            cycle_ticks: None,
            tick_bound: 0,
            out_of_atlas_tiles: 0,
            errors: Vec::new(),
        };
        let referenced: BTreeSet<usize> = animation
            .frames
            .iter()
            .map(|(target, _)| *target as usize)
            .collect();
        entry.out_of_atlas_tiles = referenced.iter().map(|o| overlay_atlas_misses(*o)).sum();
        // A frame holds at most 127 ticks (at least 1), so one cycle fits.
        entry.tick_bound = animation
            .frames
            .iter()
            .map(|(_, wait)| (*wait).max(1) as u32)
            .sum::<u32>()
            + animation.frames.len() as u32
            + 2;
        match FieldAnimation::new(scene) {
            Err(error) => entry.errors.push(error.to_string()),
            Ok(mut state) => {
                state.set_camera(animation.camera);
                let mut visited = BTreeSet::new();
                if let Err(error) = state.anim_set_active(number as i64, true) {
                    entry.errors.push(error.to_string());
                }
                for tick in 1..=entry.tick_bound {
                    visited.insert(state.animations[number].frame().max(0) as usize);
                    if let Err(error) = state.tick() {
                        entry.errors.push(format!("tick {tick}: {error}"));
                        break;
                    }
                    if state.animations[number].flags & anim_flag::HAS_NOT_FINISHED == 0 {
                        entry.cycle_ticks = Some(tick);
                        break;
                    }
                }
                entry.visited_frames = visited.into_iter().collect();
            }
        }
        if entry.cycle_ticks.is_none() && entry.errors.is_empty() {
            entry
                .errors
                .push("the animation never finished a cycle".to_owned());
        }
        if entry.out_of_atlas_tiles > 0 {
            check.failures.push(format!(
                "animation {number}: {} frame tiles fall outside the atlas",
                entry.out_of_atlas_tiles
            ));
        }
        for error in &entry.errors {
            check.failures.push(format!("animation {number}: {error}"));
        }
        check.animation_checks.push(entry);
    }
    // Frame hashes: animation 0's camera at the start, after its first step
    // and at the end of its cycle.
    if let (Some(first), true) = (
        check.animation_checks.first(),
        check.failures.is_empty() && !scene.animations.is_empty(),
    ) {
        let camera = first.camera;
        let end = first.cycle_ticks.unwrap_or(0);
        if let Some(camera_plan) = plan.cameras.iter().find(|c| c.camera == camera) {
            let mut sample = BTreeSet::new();
            sample.extend([0, 1.min(end), end]);
            for tick in sample {
                let rendered = run_all_animations(scene, 0).and_then(|mut state| {
                    state.set_camera(camera);
                    for _ in 0..tick {
                        state.tick().map_err(|error| error.to_string())?;
                    }
                    rasterize_state(camera_plan, &visit.atlas, &state)
                        .map_err(|error| error.to_string())
                });
                match rendered {
                    Ok(Some(raster)) => check.frame_hashes.push((tick, hash_hex(&raster.rgba))),
                    Ok(None) => {}
                    Err(error) => check
                        .failures
                        .push(format!("render at tick {tick}: {error}")),
                }
            }
        }
    }
    check.passed = check.failures.is_empty();
    check
}

/// `--field-animation-check`: every animated field of the install advances
/// each animation through one cycle with zero unresolved frames.
pub fn run_field_animation_check(path: &Path, out: Option<PathBuf>) -> Result<(), String> {
    let mut checks: Vec<FieldCheck> = Vec::new();
    let mut fields_without_animation = 0_usize;
    let mut problems: Vec<(String, String)> = Vec::new();
    let mut bundle_errors = Vec::new();
    for bundle in find_bundles(path) {
        let mut local_problems = Vec::new();
        let outcome = visit_bundle_fields(
            &bundle,
            None,
            |visit| {
                if visit.scene.animations.is_empty() {
                    fields_without_animation += 1;
                } else {
                    checks.push(check_field(&visit));
                }
            },
            |name, reason| local_problems.push((name.to_owned(), reason)),
        );
        problems.extend(local_problems);
        if let Err(error) = outcome {
            bundle_errors.push(format!("{}: {error}", bundle.display()));
        }
    }
    checks.sort_by(|a, b| a.field.cmp(&b.field));
    let passed = checks.iter().filter(|c| c.passed).count();
    let failed = checks.len() - passed;
    let animations: usize = checks.iter().map(|c| c.animations).sum();
    let frames: usize = checks.iter().map(|c| c.frames).sum();
    let visited: usize = checks
        .iter()
        .flat_map(|c| &c.animation_checks)
        .map(|a| a.visited_frames.len())
        .sum();
    let finished = checks
        .iter()
        .flat_map(|c| &c.animation_checks)
        .filter(|a| a.cycle_ticks.is_some())
        .count();
    let out_of_atlas: usize = checks
        .iter()
        .flat_map(|c| &c.animation_checks)
        .map(|a| a.out_of_atlas_tiles)
        .sum();
    let pass = failed == 0 && problems.is_empty() && bundle_errors.is_empty() && !checks.is_empty();
    let report = serde_json::json!({
        "total": {
            "animated_fields": checks.len(),
            "fields_without_animation": fields_without_animation,
            "passed": passed,
            "failed": failed,
            "animations": animations,
            "animation_frames": frames,
            "frames_visited_in_one_cycle": visited,
            "animations_finishing_a_cycle": finished,
            "out_of_atlas_tiles": out_of_atlas,
            "frame_hash_samples": checks.iter().map(|c| c.frame_hashes.len()).sum::<usize>(),
            "field_problems": problems.len(),
            "bundle_errors": bundle_errors.len(),
            "pass": pass,
        },
        "assumptions": [
            "Each animation is started alone with BGAACTIVE at tick 0 on its own camera; the real start comes from field scripts.",
            "A cycle ends when HasNotFinished clears (wrap, palindrome reversal or a stop frame); animations with the Loop flag are not set here.",
            "Frame hashes are SHA-256 prefixes of the animation 0 camera raster with every animation of the field started; blend modes keep the W1 assumption.",
            "Camera scroll is not modeled in placement (see field_animation.rs).",
        ],
        "field_problems": problems,
        "bundle_errors": bundle_errors,
        "fields": checks,
    });
    let target = out.unwrap_or_else(|| PathBuf::from("audit/field-animation.json"));
    std::fs::write(
        &target,
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())? + "\n",
    )
    .map_err(|error| format!("could not write {}: {error}", target.display()))?;
    println!(
        "field animation: {} animated fields ({} without animation), {passed} passed, {failed} failed, {animations} animations, {frames} frames, {visited} visited, {out_of_atlas} out-of-atlas tiles -> {}",
        checks.len(),
        fields_without_animation,
        target.display()
    );
    println!("{}", if pass { "PASS" } else { "FAIL" });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_assets::pc::bgs::{BgsAnimation, BgsCamera, BgsFile, BgsOverlay};
    use tantalus_assets::pc::field_animation::overlay_flag;

    fn overlay() -> BgsOverlay {
        BgsOverlay {
            flags: 0,
            current_z: 0,
            original_z: 0,
            width: 64,
            height: 32,
            origin: [0; 2],
            current: [0; 2],
            limits: [-100, 100, -100, 100],
            screen: [0; 2],
            scroll: [0; 2],
            fraction: [0; 2],
            camera: 0,
            is_x_offset: false,
            viewport: 0,
            sprites: Vec::new(),
        }
    }

    fn host() -> FieldAnimationHost {
        let scene = BgsFile {
            scene_length: 0,
            depth_bit_shift: 1,
            origin_z: 0,
            current_z: 0,
            origin: [0; 2],
            current: [0; 2],
            limits: [0; 4],
            screen: [0; 2],
            animations: vec![BgsAnimation {
                flags: 1,
                camera: 0,
                current_frame: 0,
                frame_rate: 256,
                counter: 0,
                frames: vec![(0, 2), (1, 1), (2, 1)],
            }],
            overlays: vec![overlay(), overlay(), overlay(), overlay()],
            lights: Vec::new(),
            cameras: vec![BgsCamera {
                projection: 100,
                rotation: [[0; 3]; 3],
                translation: [0; 3],
                center_offset: [0; 2],
                width: 320,
                height: 224,
                viewport_x: [0; 2],
                viewport_y: [0; 2],
                depth_offset: 0,
            }],
        };
        FieldAnimationHost {
            background: FieldAnimation::new(&scene).unwrap(),
            walkmesh: None,
        }
    }

    #[test]
    fn animation_events_drive_the_frame_sequence() {
        let mut host = host();
        // BGAACTIVE, then hold 2 / 1 / 1: frame index after each tick.
        assert_eq!(
            host.apply(&HostEvent::SetTileAnimationActive {
                animation: 0,
                active: true
            })
            .unwrap(),
            EventEffect::Applied
        );
        let mut frames = Vec::new();
        for _ in 0..5 {
            host.tick().unwrap();
            frames.push(host.background.animations[0].frame());
        }
        assert_eq!(frames, [0, 1, 2, 0, 0]);
        // BGARATE 512 then BGAANIME at frame 1 with the loop flag (BGAFLAG 16).
        host.apply(&HostEvent::SetTileAnimationRate {
            animation: 0,
            frame_rate: 512,
        })
        .unwrap();
        host.apply(&HostEvent::RunTileAnimation {
            animation: 0,
            start_frame: 1,
        })
        .unwrap();
        host.apply(&HostEvent::SetTileAnimationFlags {
            animation: 0,
            flags: 16,
        })
        .unwrap();
        let mut frames = Vec::new();
        for _ in 0..3 {
            host.tick().unwrap();
            frames.push(host.background.animations[0].frame());
        }
        // 1 + 2 = 3 wraps to 0; then 0 + 2 = 2 (hold 2: waits a tick).
        assert_eq!(frames, [0, 0, 2]);
    }

    #[test]
    fn overlay_events_reach_the_scroll_state() {
        let mut host = host();
        host.apply(&HostEvent::SetTileLoop {
            block: 3,
            enabled: 1,
            x_speed: 128,
            y_speed: 0,
        })
        .unwrap();
        host.apply(&HostEvent::SetTileBlockActive {
            block: 3,
            active: true,
        })
        .unwrap();
        host.tick().unwrap();
        host.tick().unwrap();
        let o = &host.background.overlays[3];
        assert_eq!(o.cur_x, 1.0);
        assert_eq!(o.flags, overlay_flag::LOOP | overlay_flag::ACTIVE);
        host.apply(&HostEvent::MoveTile {
            overlay: 3,
            offset: [200, -5, 0],
        })
        .unwrap();
        // Clamped to the +-100 limits.
        assert_eq!(host.background.overlays[3].cur_x, 100.0);
        assert!(
            host.apply(&HostEvent::SetTileBlockActive {
                block: 9,
                active: true
            })
            .is_err()
        );
    }

    #[test]
    fn walkmesh_events_need_a_walkmesh_and_other_events_are_ignored() {
        let mut host = host();
        assert_eq!(
            host.apply(&HostEvent::SetTriangleActive {
                triangle: 0,
                active: true
            })
            .unwrap(),
            EventEffect::NoWalkmesh
        );
        assert_eq!(
            host.apply(&HostEvent::GameOver).unwrap(),
            EventEffect::NotAnimation
        );
    }
}
