//! World random-encounter zones and battle scene choice.
//!
//! The per-frame trigger test is [`super::encounter_triggers`]. This module ports what
//! happens after it fires: the map id's area picks a zone, the zone's slice of the
//! 355-record encounter table is searched for the record that matches the terrain
//! and fog, and one of the record's four scenes is drawn from a pattern row
//! (`ff9.w_worldGetBattleScenePtr`, `EventEngine.SelectScene`, commit c716e1fa).
//!
//! The source has no step counter. The only accumulation is `w_frameCounter`, a
//! frame count the trigger compares against 400; see [`super::EncounterCheck`].

use std::collections::BTreeSet;

use super::coords::{id_area, id_topograph};

/// Records in the encounter table (`WMBinarayReaderExtension.ReadEncountData`).
pub const ENCOUNTER_RECORDS: usize = 355;
/// Special encounter groups and areas per group (`ReadWorldEncountSpecial`).
pub const SPECIAL_GROUPS: usize = 9;
pub const SPECIAL_AREAS: usize = 12;
/// Zones in `w_worldZoneFigure`.
pub const ZONES: usize = 26;
/// `useAlternate` offset on disc 4 (`ff9.w_worldGetBattleScenePtr`).
pub const DISC4_ALTERNATE_OFFSET: usize = 254;
/// Disc 4 only swaps records below this index.
pub const DISC4_ALTERNATE_LIMIT: usize = 100;

/// `ff9.w_worldAreaZone` (`ff9/ff9.cs:1345`), indexed by `m_GetIDArea`.
pub const AREA_ZONE: [u8; 64] = [
    0, 0, 1, 1, 2, 2, 2, 2, 3, 3, 4, 4, 5, 5, 6, 7, 7, 7, 7, 7, 7, 7, 8, 9, 9, 9, 10, 11, 11, 11,
    11, 11, 12, 12, 13, 13, 14, 14, 14, 14, 15, 15, 15, 16, 16, 17, 18, 18, 18, 18, 18, 19, 19, 19,
    19, 20, 20, 21, 21, 21, 22, 22, 23, 24,
];

/// `ff9.w_worldZoneFigure` (`ff9/ff9.cs:1411`): record pairs per zone.
pub const ZONE_FIGURE: [u8; ZONES] = [
    3, 3, 7, 4, 5, 7, 2, 6, 2, 9, 2, 7, 6, 4, 7, 10, 5, 7, 7, 6, 4, 6, 4, 2, 2, 0,
];

/// `ff9.w_worldZoneInfo` as `w_worldSystemConstructor` builds it
/// (`ff9/ff9.cs:8494`): the first record index of each zone, each zone holding
/// `figure * 2` records. Entry 25 is the end of zone 24.
pub const fn zone_info() -> [u8; ZONES] {
    let mut info = [0_u8; ZONES];
    let mut total = 0_u32;
    let mut zone = 0;
    while zone < ZONES {
        info[zone] = total as u8;
        total += ZONE_FIGURE[zone] as u32 * 2;
        zone += 1;
    }
    info
}

/// `ff9.w_worldArea2Zone`.
pub fn area_to_zone(area: i32) -> Option<usize> {
    usize::try_from(area)
        .ok()
        .and_then(|area| AREA_ZONE.get(area))
        .map(|&zone| usize::from(zone))
}

/// `EventEngine.d` (`Event/Engine/EventEngine.Static.cs:120`): cumulative
/// thresholds on a `random8` draw for scenes 0 to 2; anything above takes scene 3.
pub const SCENE_THRESHOLDS: [[u8; 4]; 4] = [
    [96, 168, 224, 255],
    [64, 128, 192, 255],
    [90, 167, 244, 255],
    [115, 217, 244, 255],
];

/// `EncountData` (`WMBinarayReaderExtension.ReadEncountData`): 10 bytes, four
/// little-endian scene ids, `pattern`, `pad`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncounterRecord {
    pub scene: [u16; 4],
    /// Bits 2 to 7 are the topograph the record applies to, bits 0 and 1 pick a
    /// row of [`SCENE_THRESHOLDS`].
    pub pattern: u8,
    /// Bit 0 is the fog flag; bits 1 to 7 are read by the source but unused.
    pub pad: u8,
}

impl EncounterRecord {
    pub fn topograph(&self) -> u8 {
        self.pattern >> 2
    }

    pub fn pattern_row(&self) -> usize {
        usize::from(self.pattern & 3)
    }

    pub fn fog(&self) -> u8 {
        self.pad & 1
    }
}

/// `SettingsState.IsFriendlyBattleOnly` as `EventEngine.SelectScene` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendlyBattleSetting {
    /// 0: normal draw.
    Off,
    /// 1: first scene of the record that is a friendly battle, else scene 0.
    Only,
    /// 2: always scene 0.
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncounterError {
    WrongRecordCount(usize),
    /// Area outside `w_worldAreaZone`.
    AreaOutOfRange(i32),
    /// The zone search ran off the table (the source would index out of range).
    NoRecord {
        zone: usize,
    },
}

impl std::fmt::Display for EncounterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for EncounterError {}

/// The 355-record table plus the zone search over it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncounterTable {
    records: Vec<EncounterRecord>,
}

impl EncounterTable {
    pub fn new(records: Vec<EncounterRecord>) -> Result<Self, EncounterError> {
        if records.len() != ENCOUNTER_RECORDS {
            return Err(EncounterError::WrongRecordCount(records.len()));
        }
        Ok(Self { records })
    }

    pub fn records(&self) -> &[EncounterRecord] {
        &self.records
    }

    /// `ff9.w_worldGetBattleScenePtr` for the actor's map id. `fog` is
    /// `w_frameFog`, `disc` is `w_frameDisc`.
    ///
    /// The first record in the zone slice whose topograph and fog match wins. On
    /// disc 4 a winning record below index 100 is replaced by the record 254 later.
    /// With no match the source returns the zone's last record.
    pub fn battle_record(
        &self,
        map_id: i32,
        fog: u8,
        disc: u8,
    ) -> Result<&EncounterRecord, EncounterError> {
        let area = id_area(map_id);
        let zone = area_to_zone(area).ok_or(EncounterError::AreaOutOfRange(area))?;
        let info = zone_info();
        let start = usize::from(info[zone]);
        let end = usize::from(info[zone + 1]);
        let topograph = id_topograph(map_id);
        for index in start..end {
            let record = self
                .records
                .get(index)
                .ok_or(EncounterError::NoRecord { zone })?;
            if i32::from(record.topograph()) == topograph && record.fog() == fog {
                let alternate = if disc == 4 && index < DISC4_ALTERNATE_LIMIT {
                    DISC4_ALTERNATE_OFFSET
                } else {
                    0
                };
                return self
                    .records
                    .get(index + alternate)
                    .ok_or(EncounterError::NoRecord { zone });
            }
        }
        end.checked_sub(1)
            .and_then(|index| self.records.get(index))
            .ok_or(EncounterError::NoRecord { zone })
    }

    /// `ff9.w_worldMapConstructor` (`ff9/ff9.cs:8708`): for every set bit of the
    /// 9-bit mask (event globals 194 and 198 as low and high byte) copy scene 2 into
    /// scene 3 of each listed record. Areas are one-based; 0 and out-of-range values
    /// (including `0xFFFF`) are skipped.
    pub fn promote_special(&mut self, mask: u16, special: &[[u16; SPECIAL_AREAS]]) {
        for (group, areas) in special.iter().enumerate().take(SPECIAL_GROUPS) {
            if (mask >> group) & 1 == 0 {
                continue;
            }
            for &area in areas {
                if let Some(record) = usize::from(area)
                    .checked_sub(1)
                    .and_then(|index| self.records.get_mut(index))
                {
                    record.scene[3] = record.scene[2];
                }
            }
        }
    }

    /// `ff9.w_friendlyBattles` (`ff9/ff9.cs:8531`): scene 3 of every record the
    /// special groups name, collected before any promotion.
    pub fn friendly_battles(&self, special: &[[u16; SPECIAL_AREAS]]) -> BTreeSet<u16> {
        let mut set = BTreeSet::new();
        for areas in special.iter().take(SPECIAL_GROUPS) {
            for &area in areas {
                if let Some(record) = usize::from(area)
                    .checked_sub(1)
                    .and_then(|index| self.records.get(index))
                {
                    set.insert(record.scene[3]);
                }
            }
        }
        set
    }
}

/// `EventEngine.SelectScene` (`Event/Engine/EventEngine.cs:191`) for a world
/// encounter. `random` is `Comn.random8()`; the source draws it only in the normal
/// branch, so callers should consume a draw only for [`FriendlyBattleSetting::Off`].
pub fn select_scene(
    record: &EncounterRecord,
    setting: FriendlyBattleSetting,
    random: u8,
    friendly: &BTreeSet<u16>,
) -> u16 {
    match setting {
        FriendlyBattleSetting::Only => record
            .scene
            .iter()
            .copied()
            .find(|scene| friendly.contains(scene))
            .unwrap_or(0),
        FriendlyBattleSetting::Never => 0,
        FriendlyBattleSetting::Off => {
            let row = &SCENE_THRESHOLDS[record.pattern_row()];
            let slot = row[..3].iter().position(|&limit| random < limit);
            record.scene[slot.unwrap_or(3)]
        }
    }
}
