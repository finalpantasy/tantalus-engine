//! Unity 5.2 `AnimationClip` (class 74) decoded from its type tree.

use super::fields::{Aabb, Fields};
use super::type_tree::Value;
use crate::install::ImportError;
use serde::{Deserialize, Serialize};

pub const ANIMATION_CLIP_CLASS_ID: u16 = 74;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe<const N: usize> {
    pub time: f32,
    #[serde(with = "serde_array")]
    pub value: [f32; N],
    #[serde(with = "serde_array")]
    pub in_slope: [f32; N],
    #[serde(with = "serde_array")]
    pub out_slope: [f32; N],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve<const N: usize> {
    pub path: String,
    pub attribute: String,
    pub class_id: i32,
    pub pre_infinity: i32,
    pub post_infinity: i32,
    pub keys: Vec<Keyframe<N>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationClip {
    pub name: String,
    pub legacy: bool,
    pub compressed: bool,
    pub use_high_quality_curve: bool,
    pub sample_rate: f32,
    pub wrap_mode: i32,
    pub bounds: Aabb,
    pub rotation_curves: Vec<Curve<4>>,
    pub position_curves: Vec<Curve<3>>,
    pub scale_curves: Vec<Curve<3>>,
    pub float_curves: Vec<Curve<1>>,
    pub compressed_rotation_curves: usize,
    pub pptr_curves: usize,
    pub muscle_clip_size: u32,
    pub muscle_start_time: f32,
    pub muscle_stop_time: f32,
    pub streamed_clip_words: usize,
    pub dense_frame_count: i32,
    pub constant_clip_values: usize,
    pub generic_bindings: usize,
    pub events: usize,
}

impl AnimationClip {
    pub fn from_value(value: &Value<'_>, source: &str) -> Result<Self, ImportError> {
        let fields = Fields::new(value, source, "AnimationClip");
        let rotation_curves = curves::<4>(&fields, "m_RotationCurves", ["x", "y", "z", "w"])?;
        let position_curves = curves::<3>(&fields, "m_PositionCurves", ["x", "y", "z"])?;
        let scale_curves = curves::<3>(&fields, "m_ScaleCurves", ["x", "y", "z"])?;
        let float_curves = curves::<1>(&fields, "m_FloatCurves", ["x"])?;
        let sample_rate = fields.f32("m_SampleRate")?;
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return Err(fields.error(format!("invalid sample rate {sample_rate}")));
        }
        let muscle = fields.sub("m_MuscleClip")?;
        let clip = muscle.sub("m_Clip")?.sub("data")?;
        let bindings = fields.sub("m_ClipBindingConstant")?;
        Ok(Self {
            name: fields.string("m_Name")?,
            legacy: fields.bool("m_Legacy")?,
            compressed: fields.bool("m_Compressed")?,
            use_high_quality_curve: fields.bool("m_UseHighQualityCurve")?,
            sample_rate,
            wrap_mode: fields.i32("m_WrapMode")?,
            bounds: fields.aabb("m_Bounds")?,
            rotation_curves,
            position_curves,
            scale_curves,
            float_curves,
            compressed_rotation_curves: fields.len("m_CompressedRotationCurves")?,
            pptr_curves: fields.len("m_PPtrCurves")?,
            muscle_clip_size: fields.u32("m_MuscleClipSize")?,
            muscle_start_time: muscle.f32("m_StartTime")?,
            muscle_stop_time: muscle.f32("m_StopTime")?,
            streamed_clip_words: clip.sub("m_StreamedClip")?.len("data")?,
            dense_frame_count: clip.sub("m_DenseClip")?.i32("m_FrameCount")?,
            constant_clip_values: clip.sub("m_ConstantClip")?.len("data")?,
            generic_bindings: bindings.len("genericBindings")?,
            events: fields.len("m_Events")?,
        })
    }

    pub fn curve_count(&self) -> usize {
        self.rotation_curves.len()
            + self.position_curves.len()
            + self.scale_curves.len()
            + self.float_curves.len()
    }

    pub fn key_count(&self) -> usize {
        self.rotation_curves
            .iter()
            .map(|c| c.keys.len())
            .sum::<usize>()
            + self
                .position_curves
                .iter()
                .map(|c| c.keys.len())
                .sum::<usize>()
            + self
                .scale_curves
                .iter()
                .map(|c| c.keys.len())
                .sum::<usize>()
            + self
                .float_curves
                .iter()
                .map(|c| c.keys.len())
                .sum::<usize>()
    }

    /// Clip length in seconds: the last key time of the legacy curves, or the
    /// muscle-clip stop time when no legacy keys exist.
    pub fn length(&self) -> f32 {
        let last = |times: &mut dyn Iterator<Item = f32>| times.fold(0.0_f32, f32::max);
        let legacy = last(
            &mut self
                .rotation_curves
                .iter()
                .flat_map(|c| c.keys.iter().map(|k| k.time))
                .chain(
                    self.position_curves
                        .iter()
                        .chain(&self.scale_curves)
                        .flat_map(|c| c.keys.iter().map(|k| k.time)),
                )
                .chain(
                    self.float_curves
                        .iter()
                        .flat_map(|c| c.keys.iter().map(|k| k.time)),
                ),
        );
        if self.key_count() > 0 {
            legacy
        } else {
            (self.muscle_stop_time - self.muscle_start_time).max(0.0)
        }
    }

    /// Frame count at the clip sample rate (length * rate, rounded, plus one).
    pub fn frame_count(&self) -> u32 {
        (self.length() * self.sample_rate).round() as u32 + 1
    }
}

fn curves<const N: usize>(
    fields: &Fields<'_, '_>,
    name: &str,
    components: [&str; N],
) -> Result<Vec<Curve<N>>, ImportError> {
    let mut out = Vec::new();
    for (number, entry) in fields.array(name)?.iter().enumerate() {
        let entry = Fields::new(entry, fields.source, "AnimationClip curve");
        let curve = entry.sub("curve")?;
        let mut keys = Vec::new();
        for key in curve.array("m_Curve")? {
            let key = Fields::new(key, fields.source, "AnimationClip key");
            let read = |field: &str| -> Result<[f32; N], ImportError> {
                if N == 1 {
                    let mut out = [0.0; N];
                    out[0] = key.f32(field)?;
                    Ok(out)
                } else {
                    key.floats(field, components)
                }
            };
            keys.push(Keyframe {
                time: key.f32("time")?,
                value: read("value")?,
                in_slope: read("inSlope")?,
                out_slope: read("outSlope")?,
            });
        }
        let mut previous = f32::NEG_INFINITY;
        for key in &keys {
            if !key.time.is_finite() || key.time < previous {
                return Err(entry.error(format!(
                    "{name}[{number}] key time {} is not finite and non-decreasing",
                    key.time
                )));
            }
            if key.value.iter().any(|value| !value.is_finite()) {
                return Err(entry.error(format!("{name}[{number}] has a non-finite key value")));
            }
            previous = key.time;
        }
        let attribute = if entry.value.field("attribute").is_some() {
            entry.string("attribute")?
        } else {
            String::new()
        };
        let class_id = if entry.value.field("classID").is_some() {
            entry.i32("classID")?
        } else {
            0
        };
        out.push(Curve {
            path: entry.string("path")?,
            attribute,
            class_id,
            pre_infinity: curve.i32("m_PreInfinity")?,
            post_infinity: curve.i32("m_PostInfinity")?,
            keys,
        });
    }
    Ok(out)
}

mod serde_array {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer, const N: usize>(
        value: &[f32; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_slice().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[f32; N], D::Error> {
        let values = Vec::<f32>::deserialize(deserializer)?;
        values
            .try_into()
            .map_err(|_| serde::de::Error::custom(format!("expected {N} floats")))
    }
}
