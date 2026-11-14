use std::time::Duration;
use tantalus_contracts::{EngineError, ErrorCategory, SimulationRate, Tick, TickInput};

pub const NORMAL_SPEED_PPM: u64 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockAdvance {
    pub inputs: Vec<TickInput>,
    pub backlogged: bool,
    pub backlog_nanoseconds: u128,
}

#[derive(Debug, Clone)]
pub struct PresentationClock {
    rate: SimulationRate,
    tick: Tick,
    accumulator_nanoseconds: u128,
    speed_ppm: u64,
    paused: bool,
    max_ticks_per_advance: u32,
}

impl PresentationClock {
    pub fn new(rate: SimulationRate, max_ticks_per_advance: u32) -> Result<Self, EngineError> {
        if rate.tick_duration_nanoseconds() == 0 {
            return Err(clock_error("tick duration must be at least one nanosecond"));
        }
        if max_ticks_per_advance == 0 {
            return Err(clock_error("clock tick budget must be nonzero"));
        }
        Ok(Self {
            rate,
            tick: Tick::default(),
            accumulator_nanoseconds: 0,
            speed_ppm: NORMAL_SPEED_PPM,
            paused: false,
            max_ticks_per_advance,
        })
    }

    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn set_speed_ppm(&mut self, speed_ppm: u64) -> Result<(), EngineError> {
        if speed_ppm == 0 {
            return Err(clock_error("speed must be nonzero; pause is explicit"));
        }
        self.speed_ppm = speed_ppm;
        Ok(())
    }

    pub fn advance(&mut self, elapsed: Duration) -> Result<ClockAdvance, EngineError> {
        if self.paused {
            return Ok(ClockAdvance {
                inputs: Vec::new(),
                backlogged: self.accumulator_nanoseconds >= self.rate.tick_duration_nanoseconds(),
                backlog_nanoseconds: self.accumulator_nanoseconds,
            });
        }

        let elapsed_nanoseconds = elapsed.as_nanos();
        let accelerated = elapsed_nanoseconds
            .checked_mul(u128::from(self.speed_ppm))
            .ok_or_else(|| clock_error("elapsed time overflow"))?
            / u128::from(NORMAL_SPEED_PPM);
        self.accumulator_nanoseconds = self
            .accumulator_nanoseconds
            .checked_add(accelerated)
            .ok_or_else(|| clock_error("clock accumulator overflow"))?;

        let tick_duration = self.rate.tick_duration_nanoseconds();
        let mut inputs = Vec::new();
        while self.accumulator_nanoseconds >= tick_duration
            && inputs.len() < self.max_ticks_per_advance as usize
        {
            let next = self
                .tick
                .checked_add(1)
                .ok_or_else(|| clock_error("tick overflow"))?;
            self.tick = next;
            self.accumulator_nanoseconds -= tick_duration;
            inputs.push(TickInput::new(next));
        }

        Ok(ClockAdvance {
            inputs,
            backlogged: self.accumulator_nanoseconds >= tick_duration,
            backlog_nanoseconds: self.accumulator_nanoseconds,
        })
    }

    pub fn drain(&mut self) -> Result<ClockAdvance, EngineError> {
        self.advance(Duration::ZERO)
    }
}

fn clock_error(context: impl Into<String>) -> EngineError {
    EngineError::new(
        ErrorCategory::InvalidProperties,
        context,
        "tantalus-core/clock",
        true,
    )
}
