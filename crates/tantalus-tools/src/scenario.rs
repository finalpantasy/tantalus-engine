use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use tantalus_assets::pc::bundled_field::BundledField;
use tantalus_contracts::{
    ButtonTransition, GameCommand, Point3, PresentationSnapshot, SimulationConfig, Tick, TickInput,
};
use tantalus_core::Simulation;
use tantalus_game::actor::FieldActorController;

#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioSpec {
    pub schema_version: u32,
    pub field: String,
    #[serde(default)]
    pub bundle: PathBuf,
    pub speed_per_tick: i32,
    pub steps: Vec<ScenarioStep>,
    pub expected: ScenarioExpectation,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioStep {
    pub ticks: u32,
    #[serde(default)]
    pub press: Vec<String>,
    #[serde(default)]
    pub release: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioExpectation {
    pub position: Point3,
    pub triangle_index: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioReport {
    pub field: String,
    pub ticks: u64,
    pub domain_events: usize,
    pub initial_position: Point3,
    pub final_position: Point3,
    pub final_triangle: usize,
    pub passed: bool,
    pub presentation: PresentationSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioError {
    pub context: String,
}

impl ScenarioError {
    fn new(context: impl Into<String>) -> Self {
        Self {
            context: context.into(),
        }
    }
}

impl Display for ScenarioError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.context)
    }
}

impl Error for ScenarioError {}

pub struct ScenarioRunner;

impl ScenarioRunner {
    pub fn read(path: &Path) -> Result<ScenarioSpec, ScenarioError> {
        let bytes = std::fs::read(path).map_err(|error| {
            ScenarioError::new(format!("could not read {}: {error}", path.display()))
        })?;
        if bytes.len() > 1024 * 1024 {
            return Err(ScenarioError::new("scenario exceeds its 1 MiB limit"));
        }
        serde_json::from_slice(&bytes).map_err(|error| {
            ScenarioError::new(format!("could not parse {}: {error}", path.display()))
        })
    }

    pub fn run(spec: &ScenarioSpec) -> Result<ScenarioReport, ScenarioError> {
        validate(spec)?;
        let loaded = BundledField::load(&spec.bundle, &spec.field).map_err(|error| {
            ScenarioError::new(format!("could not load scenario field: {error}"))
        })?;
        let mut simulation = Simulation::new(SimulationConfig::default())
            .map_err(|error| ScenarioError::new(error.to_string()))?;
        let mut actor = FieldActorController::new(
            &loaded.scene.navigation,
            tantalus_contracts::EntityId::new(1),
            spec.speed_per_tick,
        )
        .map_err(|error| ScenarioError::new(error.to_string()))?;
        let initial_position = actor.position();
        let mut domain_events = 0;
        let mut presentation = PresentationSnapshot {
            tick: Tick::default(),
            effects: Vec::new(),
            actors: vec![actor.presentation()],
            layers: Vec::new(),
            camera: Default::default(),
            mode: Default::default(),
            audio_requests: Vec::new(),
            ui_requests: Vec::new(),
        };

        for step in &spec.steps {
            for tick_in_step in 0..step.ticks {
                let mut commands = Vec::new();
                if tick_in_step == 0 {
                    for control in &step.press {
                        commands.push(GameCommand::ButtonTransition {
                            control: control.clone(),
                            transition: ButtonTransition::Press,
                        });
                    }
                    for control in &step.release {
                        commands.push(GameCommand::ButtonTransition {
                            control: control.clone(),
                            transition: ButtonTransition::Release,
                        });
                    }
                }
                let tick = Tick::new(simulation.tick().get() + 1);
                let result = simulation
                    .step(TickInput::new(tick).with_commands(commands))
                    .map_err(|error| ScenarioError::new(error.to_string()))?;
                domain_events += result.events.len();
                actor
                    .step(&simulation, 1, result.outcome)
                    .map_err(|error| ScenarioError::new(error.to_string()))?;
                presentation = result.presentation;
                presentation.actors = vec![actor.presentation()];
            }
        }

        let passed = actor.position() == spec.expected.position
            && actor.triangle_index() == spec.expected.triangle_index;
        Ok(ScenarioReport {
            field: spec.field.clone(),
            ticks: simulation.tick().get(),
            domain_events,
            initial_position,
            final_position: actor.position(),
            final_triangle: actor.triangle_index(),
            passed,
            presentation,
        })
    }
}

fn validate(spec: &ScenarioSpec) -> Result<(), ScenarioError> {
    if spec.schema_version != 1 {
        return Err(ScenarioError::new(format!(
            "unsupported scenario schema {}",
            spec.schema_version
        )));
    }
    if spec.field.is_empty() || spec.bundle.as_os_str().is_empty() {
        return Err(ScenarioError::new("scenario requires a field and bundle"));
    }
    if spec.speed_per_tick <= 0 || spec.steps.is_empty() {
        return Err(ScenarioError::new(
            "scenario requires positive speed and at least one step",
        ));
    }
    let allowed = ["move_right", "move_left", "move_down", "move_up"];
    for step in &spec.steps {
        if step.ticks == 0 {
            return Err(ScenarioError::new(
                "scenario steps must contain at least one tick",
            ));
        }
        let press = step.press.iter().collect::<BTreeSet<_>>();
        let release = step.release.iter().collect::<BTreeSet<_>>();
        if press.len() != step.press.len() || release.len() != step.release.len() {
            return Err(ScenarioError::new(
                "duplicate button transition in scenario step",
            ));
        }
        if let Some(conflict) = press.intersection(&release).next() {
            return Err(ScenarioError::new(format!(
                "button `{conflict}` is pressed and released in the same step"
            )));
        }
        for control in press.union(&release) {
            if !allowed.contains(&control.as_str()) {
                return Err(ScenarioError::new(format!(
                    "unsupported scenario control `{control}`"
                )));
            }
        }
    }
    Ok(())
}
