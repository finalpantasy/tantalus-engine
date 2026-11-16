//! Start-and-end clip sampling check with failures classified by cause.
//!
//! For every clip whose folder geo id has an assembled model: sample at
//! t=0 and at the last key time, skin the model, require every pose value
//! and skinned vertex to be finite, and require the skinned bounds to stay
//! within [`SAMPLING_TOLERANCE`] of the bind-pose bounds.

use super::pose::{self, Interpolation};
use super::{
    AssembledModel, ClipEntry, ModelCatalog, ModelDir, SerializedStore, bounds_deviation,
    sample_clip,
};
use crate::pc::unity_object::AnimationClip;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Largest allowed [`bounds_deviation`] at either sample time: every face
/// of the skinned bounds lies within one bind-pose bounds diagonal of the
/// matching bind face. A real pose (swinging limb, crouch, fall) moves a
/// face by a fraction of the body size, so one full diagonal leaves wide
/// room for extreme poses while still catching the failures that matter:
/// a wrong unit scale, a flipped or exploded skeleton, or a non-finite
/// value, each of which moves bounds by many diagonals.
pub const SAMPLING_TOLERANCE: f32 = 1.0;

const SAMPLES_PER_CAUSE: usize = 12;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ClipSamplingReport {
    pub clips: usize,
    pub tolerance: f32,
    pub tolerance_rule: String,
    pub passed: usize,
    pub failed: usize,
    /// Clips that could not be checked and are not counted as pass or fail:
    /// the clip drives no transform curves, or the model has no bind bounds.
    pub not_checkable: usize,
    /// Failure and not-checkable counts by cause.
    pub causes: BTreeMap<String, usize>,
    pub cause_samples: BTreeMap<String, Vec<String>>,
    /// Distribution of the worse of the two sampled deviations, over every
    /// clip that was skinned.
    pub deviation_bins: BTreeMap<String, usize>,
    pub max_deviation: f32,
    /// Models with the most failing clips: `label: failures of clips`.
    pub worst_models: Vec<String>,
}

fn tally(report: &mut ClipSamplingReport, cause: &str, sample: String) {
    *report.causes.entry(cause.to_owned()).or_insert(0) += 1;
    let samples = report.cause_samples.entry(cause.to_owned()).or_default();
    if samples.len() < SAMPLES_PER_CAUSE {
        samples.push(sample);
    }
}

fn poses_finite(poses: &[pose::NodePose]) -> bool {
    poses.iter().all(|p| {
        p.position
            .iter()
            .chain(&p.rotation)
            .chain(&p.scale)
            .all(|v| v.is_finite())
    })
}

/// Last key time over every transform curve of the clip.
fn clip_end_time(clip: &AnimationClip) -> f32 {
    let mut end = 0.0_f32;
    for curve in &clip.rotation_curves {
        end = end.max(curve.keys.last().map_or(0.0, |k| k.time));
    }
    for curve in clip.position_curves.iter().chain(&clip.scale_curves) {
        end = end.max(curve.keys.last().map_or(0.0, |k| k.time));
    }
    end
}

/// Samples every clip of `animations` at start and end against its paired
/// model and classifies each failure.
pub fn clip_sampling_report(
    animations: &SerializedStore<'_, '_>,
    clips: &[ClipEntry],
    catalog: &ModelCatalog,
    models: &[(ModelDir, AssembledModel)],
    mode: Interpolation,
) -> ClipSamplingReport {
    let mut report = ClipSamplingReport {
        clips: clips.len(),
        tolerance: SAMPLING_TOLERANCE,
        tolerance_rule: "max over t=0 and last key time of bounds_deviation(bind, skinned) \
            <= 1.0, where bounds_deviation is the largest min/max face difference \
            divided by the bind-pose bounds diagonal"
            .to_owned(),
        ..ClipSamplingReport::default()
    };
    let by_label: HashMap<&str, usize> = models
        .iter()
        .enumerate()
        .map(|(index, (dir, _))| (dir.label.as_str(), index))
        .collect();
    let mut per_model: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let mut max_deviation = 0.0_f32;

    for entry in clips {
        let name = format!("{}/{}", entry.geo_id, entry.name);
        let clip = match animations.clip(entry.path_id) {
            Ok(Some(clip)) => clip,
            _ => {
                report.failed += 1;
                tally(&mut report, "clip_decode_failed", name);
                continue;
            }
        };
        let paired = catalog
            .dirs_for_geo(entry.geo_id)
            .into_iter()
            .find_map(|dir| by_label.get(dir.label.as_str()).copied());
        let Some(model_index) = paired else {
            report.failed += 1;
            tally(&mut report, "no_model_in_p0data4", name);
            continue;
        };
        let (dir, model) = &models[model_index];
        let slot = per_model.entry(model_index).or_insert((0, 0));
        slot.1 += 1;
        let curve_paths: BTreeSet<&str> = clip
            .rotation_curves
            .iter()
            .map(|c| c.path.as_str())
            .chain(clip.position_curves.iter().map(|c| c.path.as_str()))
            .chain(clip.scale_curves.iter().map(|c| c.path.as_str()))
            .collect();
        if curve_paths.is_empty() {
            report.not_checkable += 1;
            tally(&mut report, "no_transform_curves", name);
            continue;
        }
        let binding = pose::bind_clip(model, &clip);
        if binding.unmatched > 0 {
            report.failed += 1;
            slot.0 += 1;
            // Same skeleton size with different names is a naming mismatch;
            // a different number of animated paths means another skeleton.
            let cause = if curve_paths.len() == model.nodes.len() {
                "curve_bone_mismatch"
            } else {
                "wrong_bone_count"
            };
            tally(
                &mut report,
                cause,
                format!(
                    "{name} vs {}: {} animated paths, {} model nodes, unmatched {}",
                    dir.label,
                    curve_paths.len(),
                    model.nodes.len(),
                    binding.unmatched_paths.join(" | ")
                ),
            );
            continue;
        }
        let Some(bind) = model.bind_bounds else {
            report.not_checkable += 1;
            tally(&mut report, "model_without_bind_bounds", name);
            continue;
        };
        let end = clip_end_time(&clip);
        let mut worst = 0.0_f32;
        let mut finite = true;
        let mut scale_mismatch = false;
        let mut skinned_any = false;
        for time in [0.0, end] {
            let poses = sample_clip(model, &clip, &binding, time, mode);
            let skinned = pose::skin_model(model, &poses);
            let ok = poses_finite(&poses)
                && skinned
                    .iter()
                    .all(|part| part.iter().all(|p| p.iter().all(|v| v.is_finite())));
            if !ok {
                finite = false;
                continue;
            }
            for (node, posed) in model.nodes.iter().zip(&poses) {
                scale_mismatch |= (0..3).any(|axis| {
                    node.local_scale[axis].abs() > 1e-9 && {
                        let ratio = posed.scale[axis] / node.local_scale[axis];
                        !(0.1..=10.0).contains(&ratio.abs())
                    }
                });
            }
            let Some(bounds) = skinned
                .iter()
                .filter_map(|points| pose::bounds_of(points))
                .reduce(pose::union_bounds)
            else {
                continue;
            };
            skinned_any = true;
            worst = worst.max(bounds_deviation(&bind, &bounds));
        }
        if !finite {
            report.failed += 1;
            slot.0 += 1;
            tally(
                &mut report,
                "non_finite_value",
                format!("{name} vs {}", dir.label),
            );
            continue;
        }
        if !skinned_any {
            report.not_checkable += 1;
            tally(&mut report, "model_without_skinned_vertices", name);
            continue;
        }
        max_deviation = max_deviation.max(worst);
        let bin = match worst {
            d if d <= 0.25 => "<=0.25",
            d if d <= 0.5 => "<=0.50",
            d if d <= 1.0 => "<=1.00",
            d if d <= 2.0 => "<=2.00",
            d if d <= 10.0 => "<=10.0",
            _ => ">10.0",
        };
        *report.deviation_bins.entry(bin.to_owned()).or_insert(0) += 1;
        if worst <= SAMPLING_TOLERANCE {
            report.passed += 1;
        } else {
            report.failed += 1;
            slot.0 += 1;
            // The known cause is a unit-conversion scale on the prefab root
            // that the clip overrides; anything else is not explained yet.
            let cause = if scale_mismatch {
                "threshold_exceeded_scale_unit_mismatch"
            } else {
                "threshold_exceeded_unknown"
            };
            tally(
                &mut report,
                cause,
                format!("{name} vs {}: {worst:.3}", dir.label),
            );
        }
    }
    report.max_deviation = max_deviation;
    let mut worst_models: Vec<(usize, usize, usize)> = per_model
        .into_iter()
        .filter(|(_, (failed, _))| *failed > 0)
        .map(|(index, (failed, total))| (index, failed, total))
        .collect();
    worst_models.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    report.worst_models = worst_models
        .into_iter()
        .take(10)
        .map(|(index, failed, total)| format!("{}: {failed} of {total}", models[index].0.label))
        .collect();
    report
}
