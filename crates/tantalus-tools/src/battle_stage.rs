//! What a fought battle looked like, for a client to draw: the scene, the
//! units at their battle positions and a per-tick record of HP and ATB.
//! Runtime only (never serialized), filled by [`crate::field_battle`].
//!
//! Positions and turns follow Memoria `btl_init.cs`:
//! - `PutMonster`: an enemy that is not a multipart slave stands at
//!   `(Xpos, -Ypos, Zpos)` turned `Euler(0, Rot & 4095, 180)`.
//! - `SetupBattlePlayer` / `SetupBattlePlayerSingle`: players stand 632
//!   apart, centred on x 0 (the first at `(count - 1) * 632 / 2`), at height
//!   0 and z `PLAYER_ORIGINAL_Z` (-1560), 400 further back in the back row
//!   (`row == 0`), turned `Euler(0, 180, 180)` for a normal start.

use tantalus_assets::pc::battle_scene::{BattleScene, Placement};
use tantalus_legacy::battle::BattleUnit;

/// `btl_init.PLAYER_ORIGINAL_Z`.
pub const PLAYER_ORIGINAL_Z: f32 = -1560.0;
/// `SetupBattlePlayer` gap between players.
pub const PLAYER_GAP: f32 = 632.0;
/// Battle logic ticks per second (`atb::BATTLE_TPS`).
pub const BATTLE_TICKS_PER_SECOND: f64 = 15.0;

/// One unit on the battlefield.
#[derive(Debug, Clone, PartialEq)]
pub enum StageModel {
    /// A party member: `CharacterId` and `CharacterSerialNumber` (the
    /// `BattleParameters.csv` row with its model and motions).
    Player { character: u8, serial: u8 },
    /// An enemy: `MonAddr[TypeNo].Geo` and the type's first motion
    /// (`FF9BattleDB.Animation` id of `mot[0]`, the idle).
    Enemy { geo: i16, idle: u16 },
    /// A multipart slave: no model of its own.
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StageUnit {
    pub model: StageModel,
    /// Battle space (Unity, y up).
    pub position: [f32; 3],
    /// Turn about the up axis in degrees (the `Euler` y).
    pub turn: f32,
    pub max_hp: u32,
    pub max_mp: u32,
}

/// The machine after one battle tick.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StageFrame {
    pub tick: u64,
    pub hp: Vec<u32>,
    pub mp: Vec<u32>,
    /// `cur.at` and `max.at` per unit.
    pub atb: Vec<(i16, i16)>,
    /// Units still in `btl_list`.
    pub present: Vec<bool>,
    /// `phase/menu-off sequence`.
    pub phase: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BattleStage {
    pub battle: i32,
    /// Bare scene name (`EF_E006`), the `BSC_` key without its prefix.
    pub scene: String,
    pub pattern: usize,
    /// `PatAddr[pattern].Camera`: default camera 0..=2, 3 and up random.
    pub camera: u8,
    /// Party first, then enemies, in machine order.
    pub units: Vec<StageUnit>,
    pub frames: Vec<StageFrame>,
    pub outcome: Option<String>,
}

impl BattleStage {
    /// The scene's units: `party` (machine units) then the pattern's
    /// placements.
    pub fn new(
        battle: i32,
        scene_name: &str,
        scene: &BattleScene,
        pattern: usize,
        party: &[BattleUnit],
    ) -> Self {
        let mut units: Vec<StageUnit> = party
            .iter()
            .enumerate()
            .map(|(slot, unit)| StageUnit {
                model: StageModel::Player {
                    character: unit.character_id,
                    serial: unit.serial_no,
                },
                position: player_position(slot, party.len(), unit.row),
                turn: 180.0,
                max_hp: unit.max_hp,
                max_mp: unit.max_mp,
            })
            .collect();
        if let Some(row) = scene.patterns.get(pattern) {
            let placements = row
                .monsters
                .iter()
                .take(usize::from(row.monster_count).min(row.monsters.len()));
            for placement in placements {
                units.push(enemy_unit(scene, placement));
            }
        }
        Self {
            battle,
            scene: scene_name.to_owned(),
            pattern,
            camera: scene.patterns.get(pattern).map_or(0, |row| row.camera),
            units,
            frames: Vec::new(),
            outcome: None,
        }
    }

    /// Records the machine units after a tick.
    pub fn record(
        &mut self,
        tick: u64,
        units: &[BattleUnit],
        max_atb: &[i16],
        present: &[bool],
        phase: &str,
    ) {
        self.frames.push(StageFrame {
            tick,
            hp: units.iter().map(|unit| unit.cur_hp).collect(),
            mp: units.iter().map(|unit| unit.cur_mp).collect(),
            atb: units
                .iter()
                .zip(max_atb)
                .map(|(unit, max)| (unit.cur_atb, *max))
                .collect(),
            present: present.to_vec(),
            phase: phase.to_owned(),
        });
    }

    /// The frame shown `seconds` into the battle.
    pub fn frame_at(&self, seconds: f64) -> Option<&StageFrame> {
        let index = (seconds.max(0.0) * BATTLE_TICKS_PER_SECOND) as usize;
        self.frames
            .get(index.min(self.frames.len().checked_sub(1)?))
    }

    /// Seconds the record lasts.
    pub fn duration(&self) -> f64 {
        self.frames.len() as f64 / BATTLE_TICKS_PER_SECOND
    }
}

/// Where party member `slot` of `count` stands; `row` 0 is the back row.
pub fn player_position(slot: usize, count: usize, row: u8) -> [f32; 3] {
    let first = (count.saturating_sub(1) as i32 * PLAYER_GAP as i32 / 2) as f32;
    let back = if row == 0 { -400.0 } else { 0.0 };
    [
        first - slot as f32 * PLAYER_GAP,
        0.0,
        PLAYER_ORIGINAL_Z + back,
    ]
}

fn enemy_unit(scene: &BattleScene, placement: &Placement) -> StageUnit {
    let kind = scene.monster_types.get(usize::from(placement.type_no));
    let slave = placement.type_no > 0 && placement.flags & Placement::FLG_MULTIPART != 0;
    StageUnit {
        model: match kind {
            Some(kind) if !slave && kind.geo >= 0 => StageModel::Enemy {
                geo: kind.geo,
                idle: kind.mot[0],
            },
            _ => StageModel::None,
        },
        position: [
            f32::from(placement.x),
            -f32::from(placement.y),
            f32::from(placement.z),
        ],
        turn: f32::from(placement.rot & 4095),
        max_hp: kind.map_or(0, |kind| kind.max_hp),
        max_mp: kind.map_or(0, |kind| kind.max_mp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_stand_632_apart_around_the_centre() {
        let xs: Vec<f32> = (0..3).map(|slot| player_position(slot, 3, 1)[0]).collect();
        assert_eq!(xs, vec![632.0, 0.0, -632.0]);
        assert_eq!(player_position(0, 4, 1), [948.0, 0.0, -1560.0]);
        // Back row stands 400 further back.
        assert_eq!(player_position(1, 2, 0), [-316.0, 0.0, -1960.0]);
    }

    #[test]
    fn frames_play_at_the_battle_tick_rate() {
        let mut stage = BattleStage::default();
        for tick in 0..30 {
            stage.record(tick, &[], &[], &[], "p");
        }
        assert_eq!(stage.duration(), 2.0);
        assert_eq!(stage.frame_at(1.0).map(|frame| frame.tick), Some(15));
        assert_eq!(stage.frame_at(9.0).map(|frame| frame.tick), Some(29));
    }
}
