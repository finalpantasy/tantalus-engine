use std::error::Error;
use std::fmt::{Display, Formatter};
use tantalus_contracts::{FieldNavigationMesh, Point3};

pub struct FieldNavigation<'a> {
    mesh: &'a FieldNavigationMesh,
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
        Ok(Self { mesh })
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
