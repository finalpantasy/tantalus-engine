//! World-map movement, camera yaw and collision as a pure, deterministic core with
//! no rendering (milestone W8). Every function cites the Memoria source it ports
//! (`Assembly-CSharp/Global/...`, commit c716e1fa).
//!
//! Arithmetic is `f32`, as in the Unity source. Sine and cosine come from the Rust
//! standard library and may differ from Unity's `Mathf` in the last bit.

pub mod camera;
pub mod coords;
pub mod encounter;
pub mod movement;
pub mod vec3;
pub mod vehicle;
pub mod walk_mesh;

pub use camera::*;
pub use coords::*;
pub use encounter::*;
pub use movement::*;
pub use vec3::Vec3;
pub use vehicle::*;
pub use walk_mesh::*;
