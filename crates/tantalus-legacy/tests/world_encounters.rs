//! World encounter zone and scene choice tests, pinned to Memoria commit c716e1fa:
//! `ff9.w_worldGetBattleScenePtr` (`Global/ff9/ff9.cs:8914`),
//! `EventEngine.SelectScene` (`Global/Event/Engine/EventEngine.cs:191`) and
//! `ff9.w_worldMapConstructor` (`Global/ff9/ff9.cs:8708`). Real-data checks read the
//! owned install and skip when it is absent.

use std::collections::BTreeSet;
use std::path::PathBuf;

use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::unity_raw::UnityRawBundle;
use tantalus_assets::pc::world_map::unity::extract_wmap_asset;
use tantalus_assets::pc::world_map::{
    DiscImage, ENCOUNTER_PACK, FRAME_PACK_SLOT, Pack, SPECIAL_ENCOUNTER_PACK, parse_encounters,
    parse_special_encounters,
};
use tantalus_legacy::world::{
    AREA_ZONE, DISC4_ALTERNATE_OFFSET, ENCOUNTER_RECORDS, EncounterError, EncounterRecord,
    EncounterTable, FriendlyBattleSetting, SCENE_THRESHOLDS, ZONE_FIGURE, ZONES, area_to_zone,
    select_scene, zone_info,
};

fn record(scene: [u16; 4], topograph: u8, row: u8, fog: u8) -> EncounterRecord {
    EncounterRecord {
        scene,
        pattern: topograph << 2 | row,
        pad: fog,
    }
}

/// A table whose record `i` carries scene ids `1000 + i` and no terrain match.
fn filler() -> Vec<EncounterRecord> {
    (0..ENCOUNTER_RECORDS as u16)
        .map(|i| record([1000 + i, 0, 0, 0], 63, 0, 0))
        .collect()
}

fn map_id(area: i32, topograph: i32) -> i32 {
    area << 8 | topograph << 2
}

#[test]
fn zone_tables_are_consistent() {
    assert_eq!(AREA_ZONE.len(), 64);
    assert_eq!(ZONE_FIGURE.len(), ZONES);
    let info = zone_info();
    assert_eq!(&info[..4], &[0, 6, 12, 26]);
    // Zones hold figure * 2 records and end exactly at the disc 4 offset.
    assert_eq!(usize::from(info[25]), DISC4_ALTERNATE_OFFSET);
    assert_eq!(ZONE_FIGURE[25], 0);
    assert_eq!(area_to_zone(0), Some(0));
    assert_eq!(area_to_zone(63), Some(24));
    assert_eq!(area_to_zone(64), None);
    assert_eq!(area_to_zone(-1), None);
}

#[test]
fn first_matching_record_in_zone_wins() {
    let mut records = filler();
    // Area 2 is zone 1 (records 6..12).
    records[8] = record([1, 2, 3, 4], 5, 0, 0);
    records[9] = record([5, 6, 7, 8], 5, 0, 0);
    records[10] = record([9, 9, 9, 9], 5, 0, 1);
    let table = EncounterTable::new(records).unwrap();
    assert_eq!(
        table.battle_record(map_id(2, 5), 0, 1).unwrap().scene,
        [1, 2, 3, 4]
    );
    // Fog flag is bit 0 of pad.
    assert_eq!(
        table.battle_record(map_id(2, 5), 1, 1).unwrap().scene,
        [9, 9, 9, 9]
    );
}

#[test]
fn no_match_returns_last_record_of_the_zone() {
    let table = EncounterTable::new(filler()).unwrap();
    // Zone 1 spans 6..12, so the fallback is record 11.
    assert_eq!(
        table.battle_record(map_id(2, 5), 0, 1).unwrap().scene[0],
        1011
    );
    // Zone 24 (area 63) spans 252..254.
    assert_eq!(
        table.battle_record(map_id(63, 5), 0, 1).unwrap().scene[0],
        1253
    );
}

#[test]
fn disc_four_uses_the_alternate_record_only_below_index_100() {
    let mut records = filler();
    records[8] = record([1, 2, 3, 4], 5, 0, 0);
    records[8 + DISC4_ALTERNATE_OFFSET] = record([11, 12, 13, 14], 5, 0, 0);
    // Zone 9 spans records 78..96 (areas 23 to 25): a hit below 100 swaps.
    records[90] = record([21, 22, 23, 24], 7, 0, 0);
    records[90 + DISC4_ALTERNATE_OFFSET] = record([31, 32, 33, 34], 7, 0, 0);
    // Zone 11 starts at 100 (areas 27 to 31), past the limit: no swap.
    records[105] = record([41, 42, 43, 44], 9, 0, 0);
    let table = EncounterTable::new(records).unwrap();
    let info = zone_info();
    assert_eq!((info[9], info[10]), (78, 96));
    assert_eq!(info[11], 100);
    assert_eq!(table.battle_record(map_id(2, 5), 0, 3).unwrap().scene[0], 1);
    assert_eq!(
        table.battle_record(map_id(2, 5), 0, 4).unwrap().scene[0],
        11
    );
    assert_eq!(
        table.battle_record(map_id(24, 7), 0, 4).unwrap().scene[0],
        31
    );
    assert_eq!(
        table.battle_record(map_id(27, 9), 0, 4).unwrap().scene[0],
        41
    );
}

#[test]
fn errors_for_bad_input() {
    assert_eq!(
        EncounterTable::new(Vec::new()).unwrap_err(),
        EncounterError::WrongRecordCount(0)
    );
    let table = EncounterTable::new(filler()).unwrap();
    // The area field is six bits, so every map id maps to a zone; the event bits
    // above it are ignored.
    assert!(table.battle_record(0xFFFF, 0, 1).is_ok());
}

#[test]
fn scene_draw_follows_the_pattern_thresholds() {
    let friendly = BTreeSet::new();
    for row in 0..4_u8 {
        let rec = record([10, 20, 30, 40], 1, row, 0);
        let t = SCENE_THRESHOLDS[usize::from(row)];
        let pick = |r| select_scene(&rec, FriendlyBattleSetting::Off, r, &friendly);
        assert_eq!(pick(0), 10);
        assert_eq!(pick(t[0] - 1), 10);
        assert_eq!(pick(t[0]), 20);
        assert_eq!(pick(t[1] - 1), 20);
        assert_eq!(pick(t[1]), 30);
        assert_eq!(pick(t[2] - 1), 30);
        assert_eq!(pick(t[2]), 40);
        assert_eq!(pick(255), 40);
    }
    assert_eq!(SCENE_THRESHOLDS[0], [96, 168, 224, 255]);
}

#[test]
fn friendly_battle_settings() {
    let rec = record([10, 20, 30, 40], 1, 0, 0);
    let friendly: BTreeSet<u16> = [30, 40].into_iter().collect();
    assert_eq!(
        select_scene(&rec, FriendlyBattleSetting::Only, 0, &friendly),
        30
    );
    assert_eq!(
        select_scene(&rec, FriendlyBattleSetting::Only, 0, &BTreeSet::new()),
        0
    );
    assert_eq!(
        select_scene(&rec, FriendlyBattleSetting::Never, 200, &friendly),
        0
    );
}

#[test]
fn special_promotion_and_friendly_set() {
    let mut records = filler();
    records[4] = record([1, 2, 3, 4], 1, 0, 0);
    records[9] = record([5, 6, 7, 8], 1, 0, 0);
    let mut table = EncounterTable::new(records).unwrap();
    let mut special = vec![[0_u16; 12]; 9];
    special[0][0] = 5; // one-based: record 4
    special[1][0] = 10; // record 9
    special[1][1] = 0xFFFF; // unused slot is skipped
    special[2][0] = 0; // zero is skipped
    let friendly = table.friendly_battles(&special);
    // Collected before promotion; skipped slots add nothing.
    assert_eq!(friendly, [4, 8].into_iter().collect());
    // Mask bit 0 only.
    table.promote_special(0b1, &special);
    assert_eq!(table.records()[4].scene, [1, 2, 3, 3]);
    assert_eq!(table.records()[9].scene, [5, 6, 7, 8]);
    table.promote_special(0b10, &special);
    assert_eq!(table.records()[9].scene, [5, 6, 7, 7]);
}

fn install() -> Option<PathBuf> {
    let dir = std::env::var_os("TANTALUS_FFIX_STREAMING_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/StreamingAssets")
        });
    dir.join("p0data3.bin").is_file().then_some(dir)
}

type RealTables = (
    Vec<tantalus_assets::pc::world_map::EncounterRecord>,
    Vec<[u16; 12]>,
);

fn load_real(disc: u8) -> Option<RealTables> {
    let dir = install()?;
    let path = dir.join("p0data3.bin");
    let bundle = UnityRawBundle::open(&path).unwrap();
    let bytes = bundle.read_entry(0, 600 * 1024 * 1024).unwrap();
    let serialized = SerializedFile::parse(&bytes, "p0data3").unwrap();
    let disc = extract_wmap_asset(
        &serialized,
        "p0data3",
        &format!("disc{disc}/discmr.img.bytes"),
    )
    .unwrap()
    .expect("discmr.img.bytes");
    let image = DiscImage::parse(&disc, "disc.img").unwrap();
    let frame = image.entry(&disc, FRAME_PACK_SLOT).unwrap();
    let pack = Pack::parse(frame, "frame pack").unwrap();
    let (records, _) =
        parse_encounters(pack.segment(ENCOUNTER_PACK).unwrap(), "encounters").unwrap();
    let (special, _) =
        parse_special_encounters(pack.segment(SPECIAL_ENCOUNTER_PACK).unwrap(), "special").unwrap();
    Some((records, special))
}

fn convert(records: &[tantalus_assets::pc::world_map::EncounterRecord]) -> EncounterTable {
    EncounterTable::new(
        records
            .iter()
            .map(|r| EncounterRecord {
                scene: r.scene,
                pattern: r.pattern,
                pad: r.pad,
            })
            .collect(),
    )
    .unwrap()
}

/// Real-data pin: what the owned install tables produce. Not compared against the
/// original runtime. Disc 1 and disc 4 each ship their own `discmr.img`.
#[test]
fn real_tables_resolve_every_area_and_terrain() {
    for disc in [1_u8, 4] {
        let Some((records, special)) = load_real(disc) else {
            eprintln!("install not found; skipping");
            return;
        };
        assert_eq!(records.len(), ENCOUNTER_RECORDS);
        let table = convert(&records);
        let friendly = table.friendly_battles(&special);
        assert!(!friendly.is_empty());
        eprintln!("disc {disc} friendly scenes: {friendly:?}");
        let mut resolved = 0_u32;
        for area in 0..64 {
            for topograph in 0..64 {
                for fog in 0..2 {
                    let rec = table
                        .battle_record(map_id(area, topograph), fog, disc)
                        .expect("zone search stays in the table");
                    for roll in [0_u8, 127, 255] {
                        let scene = select_scene(rec, FriendlyBattleSetting::Off, roll, &friendly);
                        assert!(rec.scene.contains(&scene));
                    }
                    resolved += 1;
                }
            }
        }
        assert_eq!(resolved, 64 * 64 * 2);
    }
}

/// On disc 4 the +254 records are the swap targets for records below 100: they
/// must exist, and where the swap applies the lookup returns them.
#[test]
fn real_disc_four_swap_targets_are_in_the_table() {
    let Some((records, _)) = load_real(4) else {
        eprintln!("install not found; skipping");
        return;
    };
    let table = convert(&records);
    let mut swapped = 0;
    for area in 0..64 {
        for topograph in 0..64 {
            let id = map_id(area, topograph);
            let plain = table.battle_record(id, 0, 1).unwrap();
            let disc4 = table.battle_record(id, 0, 4).unwrap();
            if plain != disc4 {
                swapped += 1;
                assert_eq!(disc4.topograph(), plain.topograph());
            }
        }
    }
    eprintln!("disc 4 swaps {swapped} area/terrain pairs");
}
