//! Minimal `UnityEngine.Vector3` subset (`f32`, same operation order as Unity).

use serde::Serialize;
use std::ops::{Add, Mul, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    pub const DOWN: Vec3 = Vec3::new(0.0, -1.0, 0.0);
    pub const UP: Vec3 = Vec3::new(0.0, 1.0, 0.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, other: Vec3) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Vec3) -> Vec3 {
        Vec3::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub fn magnitude(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// `Vector3.Normalize`: zero when the magnitude is not above `1e-5`.
    pub fn normalized(self) -> Vec3 {
        let magnitude = self.magnitude();
        if magnitude > 1e-5 {
            Vec3::new(self.x / magnitude, self.y / magnitude, self.z / magnitude)
        } else {
            Vec3::ZERO
        }
    }

    /// `Vector3.operator ==` (approximate: squared distance below `9.99999944e-11`).
    pub fn unity_eq(self, other: Vec3) -> bool {
        let d = self - other;
        d.dot(d) < 9.999_999_4e-11
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, rhs: Vec3) -> Vec3 {
        Vec3::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, rhs: Vec3) -> Vec3 {
        Vec3::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<Vec3> for f32 {
    type Output = Vec3;
    fn mul(self, rhs: Vec3) -> Vec3 {
        Vec3::new(self * rhs.x, self * rhs.y, self * rhs.z)
    }
}
