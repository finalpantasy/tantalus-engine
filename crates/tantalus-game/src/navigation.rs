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
    fn new(context: impl Into<String>) -> Self {
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
