//! W6 acceptance sweep: no visible actor with a model stands off its field's
//! walkmesh.
//!
//! Each field starts on the new-game world, its scripts run for a bounded
//! number of ticks (open dialog windows are closed every tick so scripts that
//! wait on text still progress), and the final state is checked: every actor
//! with a model and the show-model flag (`flags & 1`) must have its (x, z) on
//! some triangle of the field's walkmesh.
//!
//! Reference (Memoria `FieldMapActorController.SetPosition`): an actor whose
//! `charFlags & 1` is set is snapped onto the walkmesh (off the mesh it goes
//! to `SetDefaultCharPos`, the first triangle's center). Only actors with that
//! bit cleared sit anywhere, and the bit is cleared by `SetPathing(0)`, `POS3`
//! and `DPOS3` (`BGI_charSetActive`, `EventEngine.DoEventCode.cs`) and set
//! again by `POS`/`DPOS`. The field world tracks the bit
//! (`ActorState::path_active`) and snaps on `POS`/`DPOS`/model creation, so an
//! off-mesh actor with the bit cleared is `pathing_off` (the reference leaves
//! it there too, as is one placed while the bit was clear and re-enabled
//! after) and one placed with the bit set is `unexplained` (an engine gap).
//! An actor still at (0, 0, 0) has not been placed by its script yet and is
//! classed `unplaced`.

use std::collections::BTreeMap;

use serde::Serialize;
use tantalus_contracts::{FieldNavigationMesh, Point3};
use tantalus_game::field::FieldWorld;
use tantalus_game::navigation::FieldNavigation;
use tantalus_legacy::{EventBinary, InstructionKind};

use crate::play::PlayData;

/// One actor that was checked and is off the mesh.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OffMeshActor {
    pub actor: u64,
    pub position: [f32; 3],
    pub model: u16,
    /// `unplaced`, `pathing_off` (pathing bit cleared by the scripts) or
    /// `unexplained` (pathing on, so the reference would have snapped it).
    pub class: &'static str,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct FieldPlacement {
    pub field: i32,
    pub name: String,
    pub ticks: u64,
    pub triangles: usize,
    /// Actors with a model and the show flag.
    pub model_actors: usize,
    pub on_mesh: usize,
    pub off_mesh: Vec<OffMeshActor>,
    pub uses_set_pathing: bool,
    pub error: Option<String>,
}

/// True when (x, z) lies on a triangle of any floor of `mesh`.
pub fn on_walkmesh(
    navigation: &FieldNavigation<'_>,
    mesh: &FieldNavigationMesh,
    x: f32,
    z: f32,
) -> bool {
    let point = Point3 {
        x: x.round() as i32,
        y: 0,
        z: z.round() as i32,
    };
    let mut floors: Vec<i16> = mesh.triangles.iter().map(|t| t.floor).collect();
    floors.sort_unstable();
    floors.dedup();
    floors.into_iter().any(|floor| {
        navigation
            .project_on_floor(point, floor)
            .is_ok_and(|p| p.position.x == point.x && p.position.z == point.z)
    })
}

/// Checks every shown model actor of `world` against `mesh`; returns
/// (checked, on mesh, off mesh).
pub fn check_world(
    world: &FieldWorld,
    mesh: &FieldNavigationMesh,
) -> Result<(usize, usize, Vec<OffMeshActor>), String> {
    let navigation = FieldNavigation::new(mesh).map_err(|e| e.to_string())?;
    let (mut checked, mut on, mut off) = (0, 0, Vec::new());
    for (id, actor) in &world.actors {
        let Some(model) = actor.look.model else {
            continue;
        };
        if actor.flags & 1 == 0 {
            continue;
        }
        checked += 1;
        let [x, y, z] = actor.position;
        if on_walkmesh(&navigation, mesh, x, z) {
            on += 1;
            continue;
        }
        let class = if x == 0.0 && y == 0.0 && z == 0.0 {
            "unplaced"
        } else if !actor.path_active || actor.placed_unchecked {
            "pathing_off"
        } else {
            "unexplained"
        };
        off.push(OffMeshActor {
            actor: id.get(),
            position: actor.position,
            model,
            class,
        });
    }
    Ok((checked, on, off))
}

/// Whether any function of the binary runs `SetPathing`.
pub fn has_set_pathing(binary: &EventBinary) -> bool {
    binary.entries.iter().any(|entry| {
        binary
            .decode_entry_with_diagnostics(entry.index)
            .is_ok_and(|report| {
                report.functions.iter().any(|function| {
                    function
                        .program
                        .instructions
                        .values()
                        .any(|i| matches!(i.kind, InstructionKind::SetPathing { .. }))
                })
            })
    })
}

/// Runs one field for `ticks` ticks and checks its actors. `None` when the
/// field has no event binary or no walkmesh.
pub fn sweep_field(data: &PlayData, map: i32, ticks: u64) -> Option<FieldPlacement> {
    let binary = data.field_binary(map)?;
    let mut result = FieldPlacement {
        field: map,
        name: data.field_name(map).unwrap_or_default().to_owned(),
        uses_set_pathing: has_set_pathing(&binary),
        ..FieldPlacement::default()
    };
    let mut session = match data.start_field(map) {
        Ok(Some(session)) => session,
        Ok(None) => return None,
        Err(error) => {
            result.error = Some(error);
            return Some(result);
        }
    };
    result.triangles = session.director.world().navigation()?.triangles.len();
    for _ in 0..ticks {
        let windows: Vec<u8> = session
            .director
            .world()
            .windows
            .iter()
            .filter(|(_, open)| **open)
            .map(|(window, _)| *window)
            .collect();
        for window in windows {
            session.director.world_mut().close_window(window);
        }
        result.ticks += 1;
        match session.tick(0) {
            Ok(tick) => {
                if tick.next_field.is_some() || tick.world_map.is_some() {
                    break;
                }
            }
            Err(error) => {
                result.error = Some(error);
                break;
            }
        }
    }
    let world = session.director.world();
    let mesh = world.navigation()?;
    match check_world(world, mesh) {
        Ok((checked, on, off)) => {
            result.model_actors = checked;
            result.on_mesh = on;
            result.off_mesh = off;
        }
        Err(error) => result.error = Some(error),
    }
    Some(result)
}

#[derive(Debug, Default, Serialize)]
pub struct PlacementReport {
    pub ticks_per_field: u64,
    pub fields_in_db: usize,
    pub fields_swept: usize,
    pub fields_without_walkmesh_or_binary: usize,
    pub fields_with_error: usize,
    pub model_actors: usize,
    pub on_mesh: usize,
    pub off_mesh_by_class: BTreeMap<&'static str, usize>,
    pub fields_with_off_mesh: usize,
    /// Only fields with an off-mesh actor or an error.
    pub fields: Vec<FieldPlacement>,
}

/// Totals over per-field results.
pub fn summarize(
    ticks: u64,
    in_db: usize,
    results: Vec<Option<FieldPlacement>>,
) -> PlacementReport {
    let mut report = PlacementReport {
        ticks_per_field: ticks,
        fields_in_db: in_db,
        ..PlacementReport::default()
    };
    for result in results {
        let Some(field) = result else {
            report.fields_without_walkmesh_or_binary += 1;
            continue;
        };
        report.fields_swept += 1;
        report.model_actors += field.model_actors;
        report.on_mesh += field.on_mesh;
        if field.error.is_some() {
            report.fields_with_error += 1;
        }
        for actor in &field.off_mesh {
            *report.off_mesh_by_class.entry(actor.class).or_default() += 1;
        }
        if !field.off_mesh.is_empty() {
            report.fields_with_off_mesh += 1;
        }
        if !field.off_mesh.is_empty() || field.error.is_some() {
            report.fields.push(field);
        }
    }
    report
}

pub fn sweep_all(data: &PlayData, ticks: u64, only: Option<i32>) -> PlacementReport {
    let ids: Vec<i32> = match only {
        Some(id) => vec![id],
        None => data.field_ids(),
    };
    let results = ids
        .iter()
        .map(|&map| sweep_field(data, map, ticks))
        .collect();
    summarize(ticks, ids.len(), results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_contracts::{EntityId, NavigationTriangle};

    fn mesh() -> FieldNavigationMesh {
        let zero = Point3 { x: 0, y: 0, z: 0 };
        FieldNavigationMesh {
            vertices: vec![
                Point3 { x: 0, y: 0, z: 0 },
                Point3 { x: 100, y: 0, z: 0 },
                Point3 { x: 0, y: 0, z: 100 },
            ],
            triangles: vec![NavigationTriangle {
                vertices: [0, 1, 2],
                floor: 0,
                offset: zero,
            }],
            active_floor: 0,
            active_triangle: 0,
            actor_anchor: zero,
        }
    }

    fn actor(world: &mut FieldWorld, id: u64, position: [f32; 3], model: Option<u16>, flags: i32) {
        let entity = EntityId::new(id);
        world.add_actor(entity, position);
        let state = world.actors.get_mut(&entity).unwrap();
        state.look.model = model;
        state.flags = flags;
    }

    #[test]
    fn classifies_actors_against_a_triangle() {
        let mesh = mesh();
        let mut world = FieldWorld::default();
        actor(&mut world, 1, [10.0, 0.0, 10.0], Some(5), 1); // inside
        actor(&mut world, 2, [500.0, 0.0, 500.0], Some(5), 1); // off
        actor(&mut world, 3, [500.0, 0.0, 500.0], Some(5), 0); // hidden: skipped
        actor(&mut world, 4, [500.0, 0.0, 500.0], None, 1); // no model: skipped
        actor(&mut world, 5, [0.0, 0.0, 0.0], Some(5), 1); // corner: on the mesh
        actor(&mut world, 6, [-300.0, 0.0, -300.0], Some(7), 1); // off
        let (checked, on, off) = check_world(&world, &mesh).unwrap();
        assert_eq!((checked, on), (4, 2));
        assert_eq!(off.iter().map(|a| a.actor).collect::<Vec<_>>(), [2, 6]);
        assert!(off.iter().all(|a| a.class == "unexplained"));
        world
            .actors
            .get_mut(&EntityId::new(2))
            .unwrap()
            .placed_unchecked = true;
        let (_, _, off) = check_world(&world, &mesh).unwrap();
        assert_eq!(off[0].class, "pathing_off");
        assert_eq!(off[1].class, "unexplained");
    }

    #[test]
    fn summary_counts_and_keeps_only_failing_fields() {
        let clean = FieldPlacement {
            field: 1,
            model_actors: 2,
            on_mesh: 2,
            ..FieldPlacement::default()
        };
        let bad = FieldPlacement {
            field: 2,
            model_actors: 1,
            off_mesh: vec![OffMeshActor {
                actor: 9,
                position: [1.0, 2.0, 3.0],
                model: 4,
                class: "unexplained",
            }],
            ..FieldPlacement::default()
        };
        let report = summarize(10, 3, vec![Some(clean), Some(bad), None]);
        assert_eq!(
            (
                report.fields_swept,
                report.fields_without_walkmesh_or_binary
            ),
            (2, 1)
        );
        assert_eq!(report.model_actors, 3);
        assert_eq!(report.off_mesh_by_class["unexplained"], 1);
        assert_eq!(report.fields.len(), 1);
        assert_eq!(report.fields_with_off_mesh, 1);
    }
}
