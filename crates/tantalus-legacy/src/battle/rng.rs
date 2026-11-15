//! Injectable random source.
//!
//! The pinned code draws from `UnityEngine.Random` through three entry points:
//!
//! * `Comn.random16()` = `Random.Range(0, 65536)` (`FF9/Comn.cs:14`),
//!   also `GameRandom.Next16()` (`Memoria/Battle/Calculator/GameRandom.cs:25`).
//! * `Comn.random8()` = `Random.Range(0, 256)` (`FF9/Comn.cs:9`).
//! * `GameRandom.RandomInt(min, maxExclusive)` = `Random.Range(int, int)`
//!   (`GameRandom.cs:30`).
//! * `UnityEngine.Random.Range(0f, 1f)` for partial status resistance
//!   (`Global/btl_stat.cs:42`).
//!
//! The trait names the entry point, so a recorded sequence also pins the
//! order and kind of each draw.

/// One kind of draw. Used by [`SequenceRandom`] to check call order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Draw {
    /// `Comn.random16()`, 0..=65535.
    Random16,
    /// `Comn.random8()`, 0..=255.
    Random8,
    /// `GameRandom.RandomInt(min, maxExclusive)`.
    Range,
    /// `Random.Range(0f, 1f)`.
    Unit,
}

pub trait BattleRandom {
    /// `Comn.random16()`: uniform integer in `0..65536`.
    fn random16(&mut self) -> i32;
    /// `Comn.random8()`: uniform integer in `0..256`.
    fn random8(&mut self) -> i32;
    /// `GameRandom.RandomInt(min, max_exclusive)`.
    fn random_range(&mut self, min: i32, max_exclusive: i32) -> i32;
    /// `UnityEngine.Random.Range(0f, 1f)`.
    fn random_unit(&mut self) -> f32;
}

/// Explicit draw sequence for tests. Panics when the sequence is exhausted or
/// a draw of the wrong kind is requested, so a test fails on any change in
/// draw order or count.
#[derive(Debug, Clone, Default)]
pub struct SequenceRandom {
    draws: Vec<(Draw, i64)>,
    units: Vec<f32>,
    next: usize,
    next_unit: usize,
}

impl SequenceRandom {
    /// Integer draws, all `random16`.
    pub fn r16(values: &[i32]) -> Self {
        Self {
            draws: values.iter().map(|v| (Draw::Random16, *v as i64)).collect(),
            ..Self::default()
        }
    }

    /// Explicit kinds: `(Draw, value)` pairs.
    pub fn with_kinds(draws: &[(Draw, i32)]) -> Self {
        Self {
            draws: draws.iter().map(|(k, v)| (*k, *v as i64)).collect(),
            ..Self::default()
        }
    }

    pub fn with_units(mut self, units: &[f32]) -> Self {
        self.units = units.to_vec();
        self
    }

    /// Integer draws consumed so far.
    pub fn consumed(&self) -> usize {
        self.next
    }

    /// True when every integer draw has been used.
    pub fn is_exhausted(&self) -> bool {
        self.next == self.draws.len() && self.next_unit == self.units.len()
    }

    fn take(&mut self, kind: Draw) -> i64 {
        let Some((expected, value)) = self.draws.get(self.next).copied() else {
            panic!("draw sequence exhausted at draw {} ({kind:?})", self.next);
        };
        assert_eq!(
            expected, kind,
            "draw {} expected {expected:?}, code asked for {kind:?}",
            self.next
        );
        self.next += 1;
        value
    }
}

impl BattleRandom for SequenceRandom {
    fn random16(&mut self) -> i32 {
        let value = self.take(Draw::Random16);
        assert!((0..65536).contains(&value), "random16 out of range");
        value as i32
    }

    fn random8(&mut self) -> i32 {
        let value = self.take(Draw::Random8);
        assert!((0..256).contains(&value), "random8 out of range");
        value as i32
    }

    fn random_range(&mut self, min: i32, max_exclusive: i32) -> i32 {
        let value = self.take(Draw::Range);
        assert!(
            value >= i64::from(min) && value < i64::from(max_exclusive),
            "range draw {value} outside {min}..{max_exclusive}"
        );
        value as i32
    }

    fn random_unit(&mut self) -> f32 {
        let value = *self
            .units
            .get(self.next_unit)
            .expect("unit draw sequence exhausted");
        self.next_unit += 1;
        value
    }
}

/// Seeded generator (SplitMix64). Deterministic, no I/O. This is not the
/// Unity generator, so it does not reproduce original-game sequences.
#[derive(Debug, Clone)]
pub struct SplitMix64Random {
    state: u64,
}

impl SplitMix64Random {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl BattleRandom for SplitMix64Random {
    fn random16(&mut self) -> i32 {
        (self.next_u64() >> 48) as i32
    }

    fn random8(&mut self) -> i32 {
        (self.next_u64() >> 56) as i32
    }

    fn random_range(&mut self, min: i32, max_exclusive: i32) -> i32 {
        let span = (i64::from(max_exclusive) - i64::from(min)).max(1) as u64;
        min + (self.next_u64() % span) as i32
    }

    fn random_unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}
