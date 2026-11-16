//! Tetra Master (internal name Quad Miist) rules as a pure, deterministic core.
//!
//! No UI, no IO, no engine clock. Every random draw the Memoria source makes
//! goes through [`QuadRng`], in the source's order, so a scripted sequence
//! reproduces a game exactly. Source root (pinned Memoria c716e1fa):
//! `Assembly-CSharp/Global/Quad/Mist/Game/QuadMistGame.cs` (QMG), `Global/Card/*`,
//! `Global/ENEMY/*`, `Global/Board.cs`, `Global/Quad/Mist/Card/QuadMistCard.cs`.
//!
//! Configuration: only the default `[TetraMaster]` mode (`TripleTriad = 0`) is
//! ported. `EasyWin` and `ReduceRandom` are carried by [`Rules`]. Triple Triad
//! modes 1 and 2 (3x3 board, other formulas) are not ported.
//!
//! Static tables (not embedded here; the install owns them):
//! `EmbeddedAsset/QuadMist/MINIGAME_CARD_DATA_ADDRESS` (500 bytes, 100 cards x
//! 5), `MINIGAME_CARD_LEVEL_ADDRESS` (1024 bytes, 64 levels x 16) and
//! `MINIGAME_STAGE_ADDRESS` (512 bytes, 256 stages x 2). They are TextAssets in
//! `x64/FF9_Data/resources.assets`, read with
//! `tantalus_assets::pc::audio_decode::read_text_asset`. They are not static
//! initializers, so `managed_il.rs` is not needed for them.

pub mod ai;
pub mod card;
pub mod data;
pub mod game;
pub mod settle;

pub use ai::*;
pub use card::*;
pub use data::*;
pub use game::*;
pub use settle::*;

/// Source of `UnityEngine.Random.Range(lo, hi)` (integer overload, `hi`
/// exclusive). Implementors decide what `hi <= lo` returns; the core always
/// calls through, it never skips a draw (whether Unity advances its stream for
/// `hi == lo` is not proven).
pub trait QuadRng {
    fn range(&mut self, lo: i32, hi: i32) -> i32;
}

/// Fixed-sequence RNG for scripted games. Records every call and fails loudly
/// on exhaustion or an out-of-range value.
#[derive(Debug, Default)]
pub struct ScriptedRng {
    values: std::collections::VecDeque<i32>,
    /// `(lo, hi, value)` per call.
    pub log: Vec<(i32, i32, i32)>,
}

impl ScriptedRng {
    pub fn new(values: &[i32]) -> Self {
        Self {
            values: values.iter().copied().collect(),
            log: Vec::new(),
        }
    }
    pub fn remaining(&self) -> usize {
        self.values.len()
    }
}

impl QuadRng for ScriptedRng {
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        let v = self
            .values
            .pop_front()
            .unwrap_or_else(|| panic!("scripted rng exhausted at Range({lo}, {hi})"));
        assert!(
            (lo <= v && v < hi) || (hi <= lo && v == lo),
            "scripted value {v} outside Range({lo}, {hi}) at call {}",
            self.log.len()
        );
        self.log.push((lo, hi, v));
        v
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuadError {
    BadTable(String),
    CellOccupied(usize),
    BadCell(usize),
    BadHandIndex(usize),
    BadWise(u8),
    BadSelection(String),
}

impl std::fmt::Display for QuadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for QuadError {}

/// `Configuration.TetraMaster` switches the ported path reads
/// (`Memoria.ini [TetraMaster]`, default both false).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rules {
    /// QMG.cs:1030 `EasyWin`.
    pub easy_win: bool,
    /// QMG.cs:1033 `ReduceRandom`.
    pub reduce_random: bool,
}
