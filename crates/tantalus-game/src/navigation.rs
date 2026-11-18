use std::cell::OnceCell;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use tantalus_contracts::{FieldNavigationMesh, Point3};

pub struct FieldNavigation<'a> {
    mesh: &'a FieldNavigationMesh,
    /// Per triangle, the triangle across each edge (v0-v1, v1-v2, v2-v0):
    /// the one with an edge on the same two world points, as the BGI
    /// `neighborNdx` links do. Built on first use.
    neighbors: OnceCell<Vec<[Option<usize>; 3]>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationProjection {
    pub position: Point3,
    pub triangle_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationError {
    pub context: String,
}

impl NavigationError {
    pub(crate) fn new(context: impl Into<String>) -> Self {
        Self {
            context: context.into(),
        }
    }
}

impl Display for NavigationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.context)
    }
}

impl Error for NavigationError {}

impl<'a> FieldNavigation<'a> {
    pub fn new(mesh: &'a FieldNavigationMesh) -> Result<Self, NavigationError> {
        for triangle in &mesh.triangles {
            for vertex in triangle.vertices {
                if vertex as usize >= mesh.vertices.len() {
                    return Err(NavigationError::new(format!(
                        "navigation triangle references invalid vertex {vertex}"
                    )));
                }
            }
        }
        Ok(Self {
            mesh,
            neighbors: OnceCell::new(),
        })
    }

    fn corners(&self, index: usize) -> [Point3; 3] {
        let triangle = &self.mesh.triangles[index];
        triangle
            .vertices
            .map(|vertex| navigation_vertex(self.mesh, vertex, triangle.offset))
    }

    fn neighbors(&self) -> &[[Option<usize>; 3]] {
        self.neighbors.get_or_init(|| {
            let key = |a: Point3, b: Point3| {
                let a = (a.x, a.y, a.z);
                let b = (b.x, b.y, b.z);
                if a <= b { (a, b) } else { (b, a) }
            };
            let mut by_edge: HashMap<_, Vec<usize>> = HashMap::new();
            for index in 0..self.mesh.triangles.len() {
                let [a, b, c] = self.corners(index);
                for (p, q) in [(a, b), (b, c), (c, a)] {
                    by_edge.entry(key(p, q)).or_default().push(index);
                }
            }
            (0..self.mesh.triangles.len())
                .map(|index| {
                    let [a, b, c] = self.corners(index);
                    [(a, b), (b, c), (c, a)].map(|(p, q)| {
                        by_edge[&key(p, q)]
                            .iter()
                            .copied()
                            .find(|other| *other != index)
                    })
                })
                .collect()
        })
    }

    /// One step of a pathing actor from `from` (where it stands) to `to`,
    /// as `FieldMapActorController.ServiceChar` does it: from the triangle
    /// under `from` (nearest in height) the step crosses edges into the
    /// linked triangle (`WalkMesh.BGI_traverseTriangles`), so it stays on
    /// the floor it walks and only changes floors where triangles link (a
    /// ramp, stairs), never by dropping to another floor that shares the
    /// x/z (map 1823's spiral ramp over the lower hall). A step that
    /// leaves through an unlinked edge stops at the nearest point of the
    /// last triangle, so the walk slides along the wall. With `from` on no
    /// triangle, [`Self::nearest_on_walkmesh`] of `to`.
    pub fn step_along(&self, from: Point3, to: Point3) -> Option<NavigationProjection> {
        let Some(start) = self.snap_inside(from) else {
            return self.nearest_on_walkmesh(to);
        };
        let nearest = self.nearest_on_walkmesh(to)?;
        if self.linked_within(start.triangle_index, nearest.triangle_index, 3) {
            return Some(nearest);
        }
        Some(self.traverse(from, start.triangle_index, to))
    }

    /// Whether `to` is `from` or a triangle at most `depth` linked edges
    /// away (`FindAdjacentTrianglesRecursively`'s three levels).
    fn linked_within(&self, from: usize, to: usize, depth: usize) -> bool {
        let neighbors = self.neighbors();
        let mut frontier = vec![from];
        let mut seen = vec![from];
        for _ in 0..=depth {
            if frontier.contains(&to) {
                return true;
            }
            let next: Vec<usize> = frontier
                .iter()
                .flat_map(|index| neighbors[*index].iter().flatten().copied())
                .filter(|index| !seen.contains(index))
                .collect();
            seen.extend(&next);
            frontier = next;
        }
        false
    }

    fn traverse(&self, from: Point3, start: usize, to: Point3) -> NavigationProjection {
        let mut current = start;
        let mut previous = None;
        for _ in 0..self.mesh.triangles.len() {
            let [a, b, c] = self.corners(current);
            if let Some(position) = project_inside(a, b, c, to) {
                return NavigationProjection {
                    position,
                    triangle_index: current,
                };
            }
            let area = orient(a, b, c);
            let mut exit: Option<(f64, usize)> = None;
            for (edge, (p, q)) in [(a, b), (b, c), (c, a)].into_iter().enumerate() {
                let side = orient(p, q, to);
                if !(area > 0 && side < 0 || area < 0 && side > 0) {
                    continue;
                }
                if let Some(along) = segment_crossing(from, to, p, q)
                    && exit.is_none_or(|(best, _)| along > best)
                {
                    exit = Some((along, edge));
                }
            }
            let next = exit
                .and_then(|(_, edge)| self.neighbors()[current][edge])
                .filter(|next| Some(*next) != previous);
            match next {
                Some(next) => {
                    previous = Some(current);
                    current = next;
                }
                None => break,
            }
        }
        self.clamp_to_triangle(current, to)
    }

    /// The point of triangle `index` nearest `point` in XZ, stepped toward
    /// the triangle's center until the triangle holds it.
    fn clamp_to_triangle(&self, index: usize, point: Point3) -> NavigationProjection {
        let [a, b, c] = self.corners(index);
        let center = Point3 {
            x: ((a.x as i128 + b.x as i128 + c.x as i128) / 3) as i32,
            y: ((a.y as i128 + b.y as i128 + c.y as i128) / 3) as i32,
            z: ((a.z as i128 + b.z as i128 + c.z as i128) / 3) as i32,
        };
        let edge = [(a, b), (b, c), (c, a)]
            .into_iter()
            .filter_map(|(p, q)| nearest_on_edge(p, q, point))
            .min_by_key(|candidate| {
                let dx = candidate.x as i128 - point.x as i128;
                let dz = candidate.z as i128 - point.z as i128;
                dx * dx + dz * dz
            })
            .unwrap_or(center);
        for step in 0..=16i64 {
            let lerp = |from: i32, to: i32| {
                (i64::from(from) + (i64::from(to) - i64::from(from)) * step / 64) as i32
            };
            let candidate = Point3 {
                x: lerp(edge.x, center.x),
                y: edge.y,
                z: lerp(edge.z, center.z),
            };
            if let Some(position) = project_inside(a, b, c, candidate) {
                return NavigationProjection {
                    position,
                    triangle_index: index,
                };
            }
        }
        NavigationProjection {
            position: center,
            triangle_index: index,
        }
    }

    pub fn project(&self, point: Point3) -> Result<NavigationProjection, NavigationError> {
        self.project_on_floor(point, self.mesh.active_floor)
    }

    /// `FieldMapActorController.SetPosition` with path-checking on: of every
    /// walkmesh triangle (all floors) whose XZ footprint holds the point,
    /// take the one whose center height is nearest the requested height
    /// (`GetTriIdxAtPos`; a height of 32768 takes the highest) and put the
    /// point on it by barycentric height. A point on no triangle goes to
    /// the center of triangle 0 (`SetDefaultCharPos`).
    pub fn snap_to_walkmesh(&self, point: Point3) -> Option<NavigationProjection> {
        if let Some(inside) = self.snap_inside(point) {
            return Some(inside);
        }
        let triangle = self.mesh.triangles.first()?;
        let [a, b, c] = triangle
            .vertices
            .map(|index| navigation_vertex(self.mesh, index, triangle.offset));
        Some(NavigationProjection {
            position: Point3 {
                x: ((a.x as i128 + b.x as i128 + c.x as i128) / 3) as i32,
                y: ((a.y as i128 + b.y as i128 + c.y as i128) / 3) as i32,
                z: ((a.z as i128 + b.z as i128 + c.z as i128) / 3) as i32,
            },
            triangle_index: 0,
        })
    }

    /// The walkmesh point nearest `point` in XZ over every floor: the point
    /// itself (height from the triangle, nearest in height when several
    /// hold it) when a triangle holds it, otherwise the closest edge point.
    /// Stands in for `ServiceChar`'s clamp of a step that leaves the mesh.
    pub fn nearest_on_walkmesh(&self, point: Point3) -> Option<NavigationProjection> {
        if let Some(inside) = self.snap_inside(point) {
            return Some(inside);
        }
        let mut floors: Vec<i16> = self.mesh.triangles.iter().map(|t| t.floor).collect();
        floors.sort_unstable();
        floors.dedup();
        floors
            .into_iter()
            .filter_map(|floor| self.project_on_floor(point, floor).ok())
            .min_by_key(|p| {
                let dx = p.position.x as i128 - point.x as i128;
                let dz = p.position.z as i128 - point.z as i128;
                (dx * dx + dz * dz, p.triangle_index)
            })
            .map(|edge| self.pull_inside(edge))
    }

    /// An edge point rounded to whole units can fall a unit outside its
    /// triangle; step it toward the triangle's center until a triangle
    /// holds it.
    fn pull_inside(&self, edge: NavigationProjection) -> NavigationProjection {
        if self.snap_inside(edge.position).is_some() {
            return edge;
        }
        let triangle = &self.mesh.triangles[edge.triangle_index];
        let [a, b, c] = triangle
            .vertices
            .map(|index| navigation_vertex(self.mesh, index, triangle.offset));
        let center = Point3 {
            x: ((a.x as i128 + b.x as i128 + c.x as i128) / 3) as i32,
            y: ((a.y as i128 + b.y as i128 + c.y as i128) / 3) as i32,
            z: ((a.z as i128 + b.z as i128 + c.z as i128) / 3) as i32,
        };
        for step in 1..=16i64 {
            let lerp = |from: i32, to: i32| {
                (i64::from(from) + (i64::from(to) - i64::from(from)) * step / 64) as i32
            };
            let candidate = Point3 {
                x: lerp(edge.position.x, center.x),
                y: edge.position.y,
                z: lerp(edge.position.z, center.z),
            };
            if let Some(inside) = self.snap_inside(candidate) {
                return inside;
            }
        }
        NavigationProjection {
            position: center,
            triangle_index: edge.triangle_index,
        }
    }

    /// `FieldMapActorController.UpdateActiveTri`, which runs every frame for a
    /// pathing actor: the position is rebuilt from its barycentric ratios on the
    /// triangle under it (`Math3D.CalculateBarycentricRatioXZ`, the three
    /// sub-areas as cross-product magnitudes over the whole area), in single
    /// precision. The rebuilt x and z come back a few ulps off the whole
    /// units a scripted step lands on. That matters where a script reads
    /// `angleAsm` of the remaining distance, which truncates it to whole
    /// units: map 2204's frog Cid circles the key on exact whole units, and
    /// only these float errors let him land on the x the script waits for.
    /// The triangle is the one `GetActiveTriIdxAtPos` finds
    /// (`Math3D.PointInsideTriangleTest` with its large border), nearest in
    /// height when several hold the point. `None` off the mesh (the
    /// reference then puts the actor back where the frame started).
    pub fn rebuild_on_triangle(&self, x: f32, height: f32, z: f32) -> Option<[f32; 2]> {
        let mut best: Option<(f32, [[f32; 2]; 3])> = None;
        for triangle in &self.mesh.triangles {
            let corners = triangle
                .vertices
                .map(|index| navigation_vertex(self.mesh, index, triangle.offset));
            let flat = corners.map(|corner| [corner.x as f32, corner.z as f32]);
            if !point_inside_triangle_xz([x, z], flat) {
                continue;
            }
            let center = corners.iter().map(|corner| corner.y as f32).sum::<f32>() / 3.0;
            let distance = (height - center).abs();
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, flat));
            }
        }
        let [a, b, c] = best?.1;
        // `Vector3.Cross(u, w).magnitude` with both y set to 0: only the y
        // component, `u.z * w.x - u.x * w.z`, is left.
        let area = |u: [f32; 2], w: [f32; 2]| (u[1] * w[0] - u[0] * w[1]).abs();
        let sub = |p: [f32; 2], q: [f32; 2]| [p[0] - q[0], p[1] - q[1]];
        let whole = area(sub(a, b), sub(a, c));
        if whole == 0.0 {
            return None;
        }
        let point = [x, z];
        let (pa, pb, pc) = (sub(a, point), sub(b, point), sub(c, point));
        let ratios = [
            area(pb, pc) / whole,
            area(pc, pa) / whole,
            area(pa, pb) / whole,
        ];
        let axis = |k: usize| a[k] * ratios[0] + b[k] * ratios[1] + c[k] * ratios[2];
        Some([axis(0), axis(1)])
    }

    fn snap_inside(&self, point: Point3) -> Option<NavigationProjection> {
        let mut best: Option<(i128, usize, Point3)> = None;
        for (index, triangle) in self.mesh.triangles.iter().enumerate() {
            let [a, b, c] = triangle
                .vertices
                .map(|index| navigation_vertex(self.mesh, index, triangle.offset));
            let Some(position) = project_inside(a, b, c, point) else {
                continue;
            };
            let center = (a.y as i128 + b.y as i128 + c.y as i128) / 3;
            let distance = (point.y as i128 - center).abs();
            if best.is_none_or(|(d, _, _)| distance < d) {
                best = Some((distance, index, position));
            }
        }
        best.map(|(_, triangle_index, position)| NavigationProjection {
            position,
            triangle_index,
        })
    }

    pub fn move_by(
        &self,
        position: Point3,
        delta: [i32; 2],
    ) -> Result<NavigationProjection, NavigationError> {
        let x = position
            .x
            .checked_add(delta[0])
            .ok_or_else(|| NavigationError::new("navigation X movement overflow"))?;
        let z = position
            .z
            .checked_add(delta[1])
            .ok_or_else(|| NavigationError::new("navigation Z movement overflow"))?;
        self.project(Point3 { x, z, ..position })
    }

    pub fn project_on_floor(
        &self,
        point: Point3,
        floor: i16,
    ) -> Result<NavigationProjection, NavigationError> {
        let mut containing: Option<(i128, usize, Point3)> = None;
        for (index, triangle) in self.mesh.triangles.iter().enumerate() {
            if triangle.floor != floor {
                continue;
            }
            let [a, b, c] = triangle
                .vertices
                .map(|index| navigation_vertex(self.mesh, index, triangle.offset));
            if let Some(position) = project_inside(a, b, c, point) {
                let height_delta = (position.y as i128 - point.y as i128).abs();
                if containing
                    .as_ref()
                    .is_none_or(|(best_height, best_index, _)| {
                        (height_delta, index) < (*best_height, *best_index)
                    })
                {
                    containing = Some((height_delta, index, position));
                }
            }
        }
        if let Some((_, triangle_index, position)) = containing {
            return Ok(NavigationProjection {
                position,
                triangle_index,
            });
        }

        let mut nearest: Option<(i128, i128, usize, Point3)> = None;
        for (index, triangle) in self.mesh.triangles.iter().enumerate() {
            if triangle.floor != floor {
                continue;
            }
            let vertices = triangle
                .vertices
                .map(|index| navigation_vertex(self.mesh, index, triangle.offset));
            for (a, b) in [
                (vertices[0], vertices[1]),
                (vertices[1], vertices[2]),
                (vertices[2], vertices[0]),
            ] {
                let Some(position) = nearest_on_edge(a, b, point) else {
                    continue;
                };
                let dx = position.x as i128 - point.x as i128;
                let dz = position.z as i128 - point.z as i128;
                let distance = dx * dx + dz * dz;
                let height_delta = (position.y as i128 - point.y as i128).abs();
                if nearest
                    .as_ref()
                    .is_none_or(|(best_distance, best_height, best_index, _)| {
                        (distance, height_delta, index)
                            < (*best_distance, *best_height, *best_index)
                    })
                {
                    nearest = Some((distance, height_delta, index, position));
                }
            }
        }
        nearest
            .map(|(_, _, triangle_index, position)| NavigationProjection {
                position,
                triangle_index,
            })
            .ok_or_else(|| NavigationError::new(format!("floor {floor} has no usable triangles")))
    }
}

fn orient(a: Point3, b: Point3, point: Point3) -> i128 {
    (b.x as i128 - a.x as i128) * (point.z as i128 - a.z as i128)
        - (b.z as i128 - a.z as i128) * (point.x as i128 - a.x as i128)
}

fn project_inside(a: Point3, b: Point3, c: Point3, point: Point3) -> Option<Point3> {
    let area = orient(a, b, c);
    if area == 0 {
        return None;
    }
    let weight_a = orient(b, c, point);
    let weight_b = orient(c, a, point);
    let weight_c = orient(a, b, point);
    let inside = |weight| area > 0 && weight >= 0 || area < 0 && weight <= 0;
    if !inside(weight_a) || !inside(weight_b) || !inside(weight_c) {
        return None;
    }
    let height = (weight_a * a.y as i128 + weight_b * b.y as i128 + weight_c * c.y as i128) / area;
    Some(Point3 {
        x: point.x,
        y: height as i32,
        z: point.z,
    })
}

/// Where the segment `from`-`to` crosses the segment `p`-`q` in XZ, as the
/// fraction along `from`-`to`; `None` when they do not cross.
fn segment_crossing(from: Point3, to: Point3, p: Point3, q: Point3) -> Option<f64> {
    let delta = |a: i32, b: i32| f64::from(b) - f64::from(a);
    let (rx, rz) = (delta(from.x, to.x), delta(from.z, to.z));
    let (sx, sz) = (delta(p.x, q.x), delta(p.z, q.z));
    let denominator = rx * sz - rz * sx;
    if denominator == 0.0 {
        return None;
    }
    let (qx, qz) = (delta(from.x, p.x), delta(from.z, p.z));
    let along = (qx * sz - qz * sx) / denominator;
    let on_edge = (qx * rz - qz * rx) / denominator;
    (-1e-9..=1.0 + 1e-9).contains(&on_edge).then_some(along)
}

fn nearest_on_edge(a: Point3, b: Point3, point: Point3) -> Option<Point3> {
    let dx = b.x as i128 - a.x as i128;
    let dz = b.z as i128 - a.z as i128;
    let length_squared = dx * dx + dz * dz;
    if length_squared == 0 {
        return None;
    }
    let fraction = ((point.x as i128 - a.x as i128) * dx + (point.z as i128 - a.z as i128) * dz)
        .clamp(0, length_squared);
    let interpolate = |start: i32, delta: i128| {
        (start as i128 * length_squared + fraction * delta) / length_squared
    };
    Some(Point3 {
        x: interpolate(a.x, dx) as i32,
        y: interpolate(a.y, b.y as i128 - a.y as i128) as i32,
        z: interpolate(a.z, dz) as i32,
    })
}

fn navigation_vertex(mesh: &FieldNavigationMesh, index: u32, offset: Point3) -> Point3 {
    let vertex = mesh.vertices[index as usize];
    Point3 {
        x: vertex.x + offset.x,
        y: -(vertex.y + offset.y),
        z: vertex.z + offset.z,
    }
}

/// `Math3D.PointInsideTriangleTest` on x and z with `largeBorder`, in single precision.
fn point_inside_triangle_xz(point: [f32; 2], [a, b, c]: [[f32; 2]; 3]) -> bool {
    let dot = |u: [f32; 2], w: [f32; 2]| u[0] * w[0] + u[1] * w[1];
    let ac = [c[0] - a[0], c[1] - a[1]];
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [point[0] - a[0], point[1] - a[1]];
    let (ac_sqr, ac_ab, ac_ap) = (dot(ac, ac), dot(ac, ab), dot(ac, ap));
    let (ab_sqr, ab_ap) = (dot(ab, ab), dot(ab, ap));
    let d = 1.0 / (ac_sqr * ab_sqr - ac_ab * ac_ab);
    let bary_ab = (ab_sqr * ac_ap - ac_ab * ab_ap) * d;
    let bary_ac = (ac_sqr * ab_ap - ac_ab * ac_ap) * d;
    bary_ab >= 0.0 && bary_ac >= 0.0 && bary_ab + bary_ac <= 1.000_001
}
