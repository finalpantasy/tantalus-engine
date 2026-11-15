use tantalus_contracts::{
    ButtonTransition, EntityId, FieldNavigationMesh, GameCommand, NavigationTriangle, Point3,
    ProgressionMode, ResourceRequestId, SimulationConfig, StepOutcome, Tick, TickInput,
};
use tantalus_core::Simulation;
use tantalus_game::actor::FieldActorController;

fn mesh() -> FieldNavigationMesh {
    FieldNavigationMesh {
        vertices: vec![
            Point3::default(),
            Point3 { x: 100, y: 0, z: 0 },
            Point3 { x: 0, y: 0, z: 100 },
        ],
        triangles: vec![NavigationTriangle {
            vertices: [0, 1, 2],
            floor: 0,
            offset: Point3::default(),
        }],
        active_floor: 0,
        active_triangle: 0,
        actor_anchor: Point3::default(),
    }
}

#[test]
fn held_input_moves_each_tick_and_release_stops_movement() {
    let mesh = mesh();
    let mut simulation = Simulation::new(SimulationConfig::default()).unwrap();
    let mut actor = FieldActorController::new(&mesh, EntityId::new(1), 4).unwrap();

    let result = simulation
        .step(
            TickInput::new(Tick::new(1)).with_commands(vec![GameCommand::ButtonTransition {
                control: "move_right".to_owned(),
                transition: ButtonTransition::Press,
            }]),
        )
        .unwrap();
    actor.step(&simulation, 1, result.outcome).unwrap();
    assert_eq!(actor.position().x, 4);

    let result = simulation.step(TickInput::new(Tick::new(2))).unwrap();
    actor.step(&simulation, 1, result.outcome).unwrap();
    assert_eq!(actor.position().x, 8);

    let result = simulation
        .step(
            TickInput::new(Tick::new(3)).with_commands(vec![GameCommand::ButtonTransition {
                control: "move_right".to_owned(),
                transition: ButtonTransition::Release,
            }]),
        )
        .unwrap();
    actor.step(&simulation, 1, result.outcome).unwrap();
    assert_eq!(actor.position().x, 8);
    assert_eq!(actor.presentation().position, actor.position());
}

#[test]
fn readiness_wait_freezes_actor_without_losing_held_input() {
    let mesh = mesh();
    let config = SimulationConfig {
        progression: ProgressionMode::CompatibilityBarrier,
        ..SimulationConfig::default()
    };
    let mut simulation = Simulation::new(config).unwrap();
    let mut actor = FieldActorController::new(&mesh, EntityId::new(1), 4).unwrap();

    let result = simulation
        .step(TickInput::new(Tick::new(1)).with_commands(vec![
            GameCommand::ButtonTransition {
                control: "move_right".to_owned(),
                transition: ButtonTransition::Press,
            },
            GameCommand::RequestResource {
                request: ResourceRequestId::new(1),
            },
        ]))
        .unwrap();
    actor.step(&simulation, 1, result.outcome).unwrap();
    assert_eq!(actor.position().x, 4);

    let result = simulation.step(TickInput::new(Tick::new(2))).unwrap();
    assert_eq!(result.outcome, StepOutcome::WaitingForReadiness);
    actor.step(&simulation, 1, result.outcome).unwrap();
    assert_eq!(actor.position().x, 4);
}
