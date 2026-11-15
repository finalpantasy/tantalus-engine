use crate::navigation::{FieldNavigation, NavigationError, NavigationProjection};
use tantalus_contracts::{ActorPresentation, EntityId, FieldNavigationMesh, Point3, StepOutcome};
use tantalus_core::Simulation;

pub struct FieldActorController<'a> {
    navigation: FieldNavigation<'a>,
    entity: EntityId,
    position: Point3,
    triangle_index: usize,
    speed_per_tick: i32,
}

impl<'a> FieldActorController<'a> {
    pub fn new(
        mesh: &'a FieldNavigationMesh,
        entity: EntityId,
        speed_per_tick: i32,
    ) -> Result<Self, NavigationError> {
        if speed_per_tick <= 0 {
            return Err(NavigationError::new("actor speed must be positive"));
        }
        let navigation = FieldNavigation::new(mesh)?;
        let projected = navigation.project(mesh.actor_anchor())?;
        Ok(Self {
            navigation,
            entity,
            position: projected.position,
            triangle_index: projected.triangle_index,
            speed_per_tick,
        })
    }

    pub fn step(
        &mut self,
        simulation: &Simulation,
        stride: u32,
        outcome: StepOutcome,
    ) -> Result<NavigationProjection, NavigationError> {
        if outcome != StepOutcome::Advanced {
            return Ok(self.projection());
        }
        let x = axis(simulation, "move_right", "move_left");
        let z = axis(simulation, "move_down", "move_up");
        if x == 0 && z == 0 {
            return Ok(self.projection());
        }
        let stride = i32::try_from(stride)
            .map_err(|_| NavigationError::new("actor movement stride is too large"))?;
        let projected = self.navigation.move_by(
            self.position,
            [
                x * self.speed_per_tick * stride,
                z * self.speed_per_tick * stride,
            ],
        )?;
        self.position = projected.position;
        self.triangle_index = projected.triangle_index;
        Ok(projected)
    }

    pub fn position(&self) -> Point3 {
        self.position
    }

    pub fn triangle_index(&self) -> usize {
        self.triangle_index
    }

    pub fn presentation(&self) -> ActorPresentation {
        ActorPresentation {
            entity: self.entity,
            position: self.position,
            size: Point3 { x: 16, y: 24, z: 1 },
            color: [220, 190, 110, 255],
            animation: None,
        }
    }

    fn projection(&self) -> NavigationProjection {
        NavigationProjection {
            position: self.position,
            triangle_index: self.triangle_index,
        }
    }
}

fn axis(simulation: &Simulation, positive: &str, negative: &str) -> i32 {
    i32::from(simulation.button_held(positive)) - i32::from(simulation.button_held(negative))
}
