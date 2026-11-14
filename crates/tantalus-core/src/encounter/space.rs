use serde::{Deserialize, Serialize};
use tantalus_contracts::{EngineError, ErrorCategory, Point3};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterSpaceKind {
    OriginalArena,
    CurrentScene,
    PreparedArena,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceTransform {
    pub scale_millionths: u32,
    pub translation: Point3,
}

impl SpaceTransform {
    pub fn identity() -> Self {
        Self {
            scale_millionths: 1_000_000,
            translation: Point3::default(),
        }
    }

    pub fn map(self, point: Point3) -> Result<Point3, EngineError> {
        if self.scale_millionths == 0 {
            return Err(space_error("arena scale must be nonzero"));
        }
        let scale = i128::from(self.scale_millionths);
        Ok(Point3 {
            x: map_component(point.x, scale, self.translation.x)?,
            y: map_component(point.y, scale, self.translation.y)?,
            z: map_component(point.z, scale, self.translation.z)?,
        })
    }
}

fn map_component(value: i32, scale: i128, translation: i32) -> Result<i32, EngineError> {
    let scaled = i128::from(value)
        .checked_mul(scale)
        .ok_or_else(|| space_error("arena coordinate overflow"))?
        / 1_000_000;
    let translated = scaled
        .checked_add(i128::from(translation))
        .ok_or_else(|| space_error("arena translation overflow"))?;
    i32::try_from(translated)
        .map_err(|_| space_error("arena coordinate exceeds the fixed-point range"))
}

fn space_error(context: impl Into<String>) -> EngineError {
    EngineError::new(
        ErrorCategory::InvalidProperties,
        context,
        "tantalus-core/encounter/space",
        true,
    )
}
