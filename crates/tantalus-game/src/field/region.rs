//! Field regions (reference `Quad` objects) and the player-in-region query.
//!
//! Ported from Memoria `EventEngine.TreadQuad.cs` (`TreadQuad`, `IsInQuad`),
//! `Math3D.PointInsideTriangleTest` (large-border variant) and the `QUAD`
//! opcode (`SetRegion`, `EventEngine.DoEventCode.cs:918`), which stores the
//! polygon of the current region object as signed 16-bit X/Z vertices.
//!
//! Each frame the reference calls `EventCollision.CollisionRequest` for the
//! controlled actor while user control is on (`EventEngine.ProcessEvents.cs`),
//! which calls `TreadQuad(po, 2)` and, for the first active region that has a
//! tag-2 function and contains the actor's X/Z, `Request(quad, 1, 2)`. This
//! module answers the containment part; choosing the function and the request
//! level is the caller's.
//!
//! Not modeled: Memoria's `QuadCircleData` per-map hotfix circles
//! (`IsInQuadHotFix`), a Memoria-only correction for a few fields.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tantalus_contracts::EntityId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    /// `Quad.q[i]` as `(X, Z)`.
    pub vertices: Vec<[i16; 2]>,
}

impl Region {
    /// `IsInQuad`: true when the point lies in any triangle formed by three
    /// consecutive vertices (indices `i, i+1, i+2` modulo `n`).
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let n = self.vertices.len();
        (0..n).any(|i| {
            let a = self.vertices[i % n];
            let b = self.vertices[(i + 1) % n];
            let c = self.vertices[(i + 2) % n];
            point_in_triangle([x, z], a, b, c)
        })
    }
}

fn point_in_triangle(p: [f32; 2], a: [i16; 2], b: [i16; 2], c: [i16; 2]) -> bool {
    let a = [f32::from(a[0]), f32::from(a[1])];
    let b = [f32::from(b[0]), f32::from(b[1])];
    let c = [f32::from(c[0]), f32::from(c[1])];
    let dot = |u: [f32; 2], v: [f32; 2]| u[0] * v[0] + u[1] * v[1];
    let ac = [c[0] - a[0], c[1] - a[1]];
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let ac_sqr = dot(ac, ac);
    let ac_ab = dot(ac, ab);
    let ac_ap = dot(ac, ap);
    let ab_sqr = dot(ab, ab);
    let ab_ap = dot(ab, ap);
    let d = 1.0 / (ac_sqr * ab_sqr - ac_ab * ac_ab);
    let bary_ab = (ab_sqr * ac_ap - ac_ab * ab_ap) * d;
    let bary_ac = (ac_sqr * ab_ap - ac_ab * ac_ap) * d;
    // Degenerate triangles give NaN or infinity and fail these comparisons,
    // as in the reference.
    bary_ab >= 0.0 && bary_ac >= 0.0 && bary_ab + bary_ac <= 1.000_001
}

/// Regions containing `(x, z)`, in entity order (the reference walks the
/// active object list and takes the first match that has the wanted tag).
pub fn regions_at(regions: &BTreeMap<EntityId, Region>, x: f32, z: f32) -> Vec<EntityId> {
    regions
        .iter()
        .filter(|(_, region)| region.contains(x, z))
        .map(|(entity, _)| *entity)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Region {
        Region {
            vertices: vec![[-100, -100], [100, -100], [100, 100], [-100, 100]],
        }
    }

    #[test]
    fn convex_polygon_contains_interior_and_border_points() {
        let region = square();
        assert!(region.contains(0.0, 0.0));
        assert!(region.contains(99.0, -99.0));
        assert!(region.contains(100.0, 100.0));
        assert!(!region.contains(101.0, 0.0));
        assert!(!region.contains(0.0, -150.0));
    }

    #[test]
    fn degenerate_regions_contain_nothing() {
        let empty = Region { vertices: vec![] };
        assert!(!empty.contains(0.0, 0.0));
        let line = Region {
            vertices: vec![[0, 0], [10, 0], [20, 0]],
        };
        assert!(!line.contains(5.0, 0.0));
    }

    #[test]
    fn query_lists_every_containing_region_in_entity_order() {
        let mut regions = BTreeMap::new();
        regions.insert(EntityId::new(7), square());
        regions.insert(
            EntityId::new(3),
            Region {
                vertices: vec![[0, 0], [300, 0], [300, 300], [0, 300]],
            },
        );
        assert_eq!(
            regions_at(&regions, 50.0, 50.0),
            vec![EntityId::new(3), EntityId::new(7)]
        );
        assert_eq!(regions_at(&regions, 200.0, 200.0), vec![EntityId::new(3)]);
        assert!(regions_at(&regions, -500.0, 0.0).is_empty());
    }
}
